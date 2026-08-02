# RT Dispatch Architecture (native Rust worker + explicit Python oracle)

Status: CURRENT — Rust is the preferred eligible real-time Win32 sender path; Python remains the
deterministic oracle and an explicit diagnostic backend. Backend selection and fidelity strictness
are separate policies.
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
 ├─ SendLatencyEstimator                             p95 latency buckets by kind/polyphony
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

**Native control-path clock contract.** `QpcTicks`, `TimelineTicks`, and `DurationTicks` are the
source of truth for production scheduling. Deadline mapping, minimum hold, pause/rebase,
focus recovery, retry/recovery offsets, supervisor leases, cold classification, and wait/spin
targets stay in checked tick arithmetic. Microseconds are accepted or emitted only at the
Python/configuration, estimator, JSON, and human-readable telemetry boundaries; the worker does
not round-trip ticks through microseconds during a control-path decision. SendInput outcomes
retain their exact start/completion QPC ticks until telemetry serialization.

**Resumable suspension.** Focus loss and manual pause release and verify physical keys, then use
`cancel_live_generations()`: `Active` and `ReleasePending` become cancelled while future
`Scheduled` generations remain intact. Authored Up events belonging to cancelled generations are
suppressed as stale. Terminal `cancel_all()` is used only when the session cannot resume, such as
quit, skip, panic, supervisor lease expiry, timing/input-integrity error, or final termination.
Both paths validate coordinator invariants and cleanup state; cleanup cannot turn a pre-existing
ledger/mask mismatch into a successful result.

- **Onset = dispatch completion.** The adaptive lead uses a bounded rolling p95 of
  `send_duration_us` for each Down/Up polyphony bucket, with a monotonic envelope across chord
  sizes. Five samples warm a bucket; cold buckets use a conservative static prior plus the last
  lower bucket/global p95. Lead is capped at the configured maximum and the estimator exports
  saturation/residual evidence through runtime telemetry. `lateness_us` may legitimately be
  negative; `visible_lateness_us` is the on-time metric.
- **Pending-release cohort planning is fixed-point.** Up lead selection counts only the pending
  releases in the next effective-release cohort, not every pending key. The bounded plan carries
  deadline, lead, polyphony and saturation state and is reused for both the wait deadline and the
  pop operation, so a later release cannot over-lead an earlier one-key release.
- **Lead is symmetric** (downs and pending releases) and floor-clamped: a release becomes due at
  `max(scheduled_release − lead_up, release_not_before)` where
  `release_not_before = down_dispatch_completed + min_hold` — the 1-frame floor always wins.
- **No-early-conflict guard**: a down batch is never popped before its authored time while any of
  its scan codes is active or pending release (an early pop would become a dropped note).
  `next_authored_us` is guard-aware so a blocked batch reports its authored time as the deadline
  (no busy-loop while waiting for the blocking release).
- **Deadline mapping is single-sample.** The worker samples QPC ticks once, derives logical
  elapsed time from that same sample, and maps the remaining logical interval to an absolute QPC
  target. Bookkeeping between two independent samples must not be charged as extra deadline
  lateness. The physical playback anchor is placed in the future by the initial lead plus a wake
  guard, so an authored first note at `t=0` can dispatch at `anchor - lead` rather than being
  forced late by the worker prologue.
- **Chord conflict default is fidelity-first.** `drop_chord` rejects the whole authored chord;
  `strict` also terminates playback; `degraded` is an explicit legacy/diagnostic mode that may
  send a partial chord. Multiple same-timestamp Down batches are rejected by the native compiler
  because they cannot be made atomic after the Python boundary. At the Rust/PyO3 boundary,
  `strict_timing = true` always coerces the effective policy to `AbortPlayback`, even when a
  caller omits the policy and receives the Python-facing `drop_chord` default.
- **Strict completion SLO is post-send.** A clean, single-attempt Down and a clean,
  non-deferred, single-source Up must complete within their configured
  `strict_down_completion_late_us` / `strict_up_completion_late_us` thresholds (2,000 µs
  defaults). Exceeding either threshold records `strict_completion_slo_exceeded`, performs full
  cleanup, and ends with a controlled error. Deferred or mixed release cohorts are excluded from
  this comparison because their authored timestamp is not their effective dispatch target.
- **Estimator query cost is bounded.** Rolling p95 values are refreshed only when a sample is
  inserted or state is imported; real-time lead queries are O(polyphony) integer comparisons and
  do not sort the rolling window. Saturation is returned with the applied lead from the same
  estimate.
- **Wake-error probe** (`enable_adaptive_spin`): 30 × 2 ms probe sleeps run strictly *before*
  `start_perf` (same rule as `gc.collect`), deriving
  `effective_spin_threshold = clamp(spin_floor_us, 3000, p95_wake_error + 200)` µs (default
  `spin_floor_us = 700`, cap 3000 µs). The probe also retains p50/p99/max diagnostics. A later
  reprobe uses a robust `median + 6 × MAD + 200 µs` candidate over its small cooperative sample,
  raises the threshold immediately, and lowers it by at most 50 µs per update. This keeps one timer
  outlier from forcing a 3 ms spin window while retaining fast protection when the timer path degrades.
- **Cross-session lead cache:** `SendLatencyEstimator` exports/imports version-4 rolling p95
  samples plus separate Up residual state via `.cache/lead_estimator.json`; version-2 and
  version-3 caches are migrated conservatively. Wrapped ring windows are exported oldest-first so
  restore preserves the next overwrite position.
  Corrupt/version-mismatched cache is silently dropped. Loaded flag is recorded in
  `runtime_options.lead_cache_loaded`.
- **Idle-gap core warmup (Phase 1/6):** When the gap since last `SendInput` completion ≥ 20 ms, a
  `core_warmup_budget_us` (default 200 µs, capped at 500 µs) is added directly to the
  `effective_spin_threshold` for the final precision wait. The cold/hot decision and this guard
  use physical QPC time, not the logical playback clock, so a long pause/focus recovery still
  treats the first subsequent send as cold. This expands the busy-spin window right before the
  deadline to warm the CPU core without adding a separate blocking sleep cycle.
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

**Calibration evidence boundary.** The latency calibration cache is schema version 6,
measurement protocol 3. Measured bucket admission uses the actual QPC idle gap from the
immediately previous exact SendInput completion to the current exact SendInput entry; a
requested sleep overshoot is a class mismatch and is rejected from timing quantiles. It is an
`injected_raw_input_delivery_proxy`: a dedicated native calibration process injects through
Windows `SendInput` into an app-owned window and observes its `WM_INPUT` delivery. The player
process never changes its own Raw Input registration for calibration. It does not measure Sky
process polling, render-frame timing, or audio onset. `sampled_at` is UTC metadata and
`evidence_kind` identifies this boundary; no freshness TTL is applied by the loader. Warm-up
injections are tracked separately and excluded from measured classes. Only complete,
anomaly-free samples enter timing quantiles; partial, timeout, reordered, duplicate, or unexpected
receipts remain diagnostic counters. The calibration process snapshots/restores its registration
and performs bounded full-instrument KeyUp cleanup; uncertain cleanup or a failed subprocess is
an error, not successful calibration. Full calibration is a sequential 24-bucket run split into
1,000-sample native chunks. A cold chunk has a 100-second idle-gap floor and a 180-second
per-process timeout; prior chunks are atomically checkpointed with a SHA-256 before the next
physical-input chunk starts. Raw sample evidence is retained so chunk quantiles are merged
exactly. Resume is fail-closed on any mismatch in exact Git SHA, native build/source fingerprint,
clean-worktree state, toolchain, host fingerprint, schema/protocol, or full configuration.
Diagnostic runs are single-bucket, progress-reporting, and always emit an ineligible artifact or
failure report containing the exact bucket/sample/phase/error/Win32/cleanup context. Only the
finalizer may write trusted cache evidence, and it requires all 24 known buckets, 5,000 samples
per bucket, exact aggregate totals, identical provenance, and successful cleanup. A dirty,
diagnostic, incomplete, or SHA-mismatched artifact is not release evidence. The sender telemetry
stream instead uses `evidence_kind = "sender_completion"` and never claims Raw Input or
game-observed delivery.

**Clock failure policy.** QPC is the sole real-time clock domain after native preparation. A failed
runtime QPC query, including a query immediately after a successful `SendInput`, is terminal: the
worker records the timing-integrity error, stops authored dispatch, performs bounded full-instrument
cleanup, and publishes an error outcome. It must not substitute timestamp zero or continue with a
microsecond round-trip.

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

`DispatchPolicy.backend` is `auto`, `rust`, or `python`. `auto` selects Rust for the eligible
real Windows backend and selects Python for an explicit unsupported feature such as nonzero chord
stagger; missing/incompatible native metadata remains fail-closed. `rust` always fails closed on
any admission or capability mismatch. `python` is an explicit diagnostic/oracle choice.
`DispatchPolicy.fidelity` is independently `normal` or `strict`: normal keeps integrity failures
terminal but records isolated timing tails, while strict also applies completion SLO aborts.
The selected backend, reason, probe diagnostics and fidelity mode are shown in runtime telemetry.
The Python supervisor owns a `try/finally` cleanup path, and the Rust worker has a bounded
supervisor lease that performs full-instrument release if heartbeats stop.

## 6. Production defaults & kill switches

All graduated ON (config/RUNTIME_STATE layer; library/engine constructor defaults stay off so
deterministic tests are unaffected):

| Feature | Default | Kill switch |
|---|---|---|
| Dispatch backend | `dispatch_backend: auto` (Rust preferred when eligible) | `dispatch_backend: python` / `SKY_USE_PYTHON_DISPATCH=1` |
| Fidelity policy | `fidelity_mode: normal` | `fidelity_mode: strict` for acceptance/soak |
| MMCSS/priority ladder | `rt_priority_mode: auto` (config) | `--rt-priority-mode off` |
| Adaptive dispatch lead | `enable_adaptive_lead: true` (config) | `--no-adaptive-lead` |
| Same-key chord conflict | `drop_chord` for best-effort; Rust strict timing coerces to abort | `degraded` legacy or explicit `strict` abort |
| Lead p95 cross-session cache | `.cache/lead_estimator.json` | `lead_cache_path = None` |
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

The Rust path reserves the configured bounded retain-first record buffer before the playback epoch
when telemetry is enabled, so a long diagnostic session cannot trigger a Vec growth/copy on the
dispatch thread. Telemetry is opt-in and this predictable memory reservation is preferred over
allocator jitter; with telemetry disabled no event record is materialized. The hard cap remains
explicit. The compiled
schedule uses a flat 8-byte packed `CompactIntent` arena plus small batch headers; only the current batch is
materialized into full intent views. Runtime active/release ownership uses 15-key arrays and
bitmasks rather than hash tables. Per-record scan/generation fields use inline storage and
reasons remain compact IDs on the worker; bounded summary counters and 50 µs histograms remain
available in every native run, including summary-only runs where the retain-first event buffer is
disabled. Reason strings and JSON are materialized only after terminal join. This preserves the
existing CSV fields and outcome strings without per-send disk I/O while making production health
histograms available without retaining every event.

The unsigned logical timeline also guards against sub-lead collapse: authored timestamps smaller
than the requested lead are not all saturated to deadline zero. The first action uses the future
physical startup anchor; later sub-lead actions temporarily dispatch without early lead so their
relative ordering remains observable.
Normal adaptive lead uses the rolling p95; native strict timing selects the clamped rolling upper
tail and keeps the global upper-tail guard for sparse local buckets. Strict mode aborts after the
configured repeated positive-at-cap condition or any clean completion SLO violation. Normal mode
still aborts integrity, cleanup, focus and worker-health failures but records isolated timing tails.

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
completion-error p50/p95/p99/max, signed/absolute/early/late distributions split by Down/Up
and polyphony, spin CPU time, peak RSS, command-interrupt latency, drop/release and
zero-progress counters, and outcome. This mock-backend harness does not measure game
sampling, frame observation, audio onset, or real `SendInput` delivery; those remain
separate Windows test-window evidence gates.

The native accuracy-first path requires `chord_stagger_us == 0`. A nonzero stagger is retained
for the Python diagnostic/remote-listener path. In `auto` the selector records an explicit Python
fallback reason; forcing Rust reports an unsupported-capability error instead of silently changing
the logical chord into multiple syscalls.
