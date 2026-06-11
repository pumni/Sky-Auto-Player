# RT Pipeline Extreme Optimization Plan

> [!WARNING]
> **ARCHIVED 2026-06-11 — COMPLETED.** All 7 phases executed and accepted the same day.
> Outcome stamps: Phases 0–6 implemented by the executor; acceptance round 1 found 8 blockers,
> all fixed in round 2 (gates: 366/366 tests, pyright at HEAD baseline, micro-bench −52%/−79%/−22%).
> Live A/B on `blue` @local-precise/144: adaptive lead moved median down-onset error from
> +420 µs to **−3 µs**; event-wait and adaptive-spin collapsed lateness p99 from 553–1446 µs to
> 104/80 µs; zero drops everywhere; 1-frame floor verified intact. The dedicated priority-ladder
> arm was invalidated twice by a stale-config overwrite (legacy `rt_time_critical` migration bug,
> since removed) and MMCSS was graduated on user decision with `auto` default + kill switch.
> Phase 7 graduation: all features default ON at the config/RUNTIME_STATE layer with `--no-*`
> kill switches; engine constructor defaults stay off for deterministic tests.
> Current architecture: [rt-dispatch-architecture.md](../rt-dispatch-architecture.md).
> Numbers: [perf-baselines/2026-06-baseline.md](../perf-baselines/2026-06-baseline.md).

**Status:** COMPLETED (archived)
**Date:** 2026-06-11
**Executor:** any capable AI agent (follow this document phase by phase; do not improvise beyond it)
**Acceptance reviewer:** Claude session acting on behalf of the user — every phase MUST pass the
acceptance gates in §8 before the next phase starts. The reviewer, not the executor, declares a
phase done.

This plan covers the scheduling core and key-delivery path only:
`domain/scheduler*.py` → `orchestration/runtime_dispatch.py` → `orchestration/engine.py` →
`infrastructure/{backend,realtime,timing,focus}.py` → `platform/win32/inputs.py`.
Windows-only product; non-win32 paths exist solely for tests.

---

## 0. The single ground truth

> **The game registers a key press if and only if the key is observed held for at least 1 game
> frame.** (Frame-bound sampling; empirically validated in-game. See
> `docs/timing-principles.md` §0, §3.)

Every optimization in this plan is subordinate to that truth. Any change that can make an
observed (completion-to-completion) hold fall below 1 frame is **wrong by definition**, no matter
what it does to latency numbers.

### 0.1 Non-negotiable invariants (I1–I9)

| # | Invariant | Enforced where today |
|---|---|---|
| I1 | Game-observed hold ≥ `min_hold_us`; release floor anchored to **down-dispatch completion**: `release_not_before = down_dispatch_completed + min_hold` | `runtime_dispatch.py:232`, tests `test_release_guard_anchors_hold_to_down_dispatch_completion`, `test_observed_hold_never_below_one_frame_under_asymmetric_send_latency`, `test_hold_floor_preserved_when_thread_stalls_during_hold` |
| I2 | One-frame math uses **ceil**: `frame_us = ceil(1e6/fps)`; never revert to `round()`/1.05-style fudge | `scheduler_types.py:171`, `domain/validation.py` |
| I3 | Lateness never rebases the musical timeline (no cumulative drift; absolute schedule) | `test_late_burst_never_shifts_the_absolute_music_timeline`, `test_repeated_stalls_do_not_accumulate_musical_slowdown` |
| I4 | Up-before-down ordering at equal timestamps; same-key conflict ⇒ controlled drop, never a stuck key | scheduler stage 4; coordinator `split_down_intents` |
| I5 | Input simulation is **SendInput only**; no game-file/memory/anti-cheat interaction (AGENTS.md hard constraints) | `platform/win32/inputs.py` |
| I6 | Scheduler (`build_key_actions`) stays pure — no wall clock, no platform calls | `domain/scheduler.py` |
| I7 | **No process-wide priority class change.** Per-thread scheduling (MMCSS / SetThreadPriority on the dispatch thread only) is allowed (user decision 2026-06-11) | new in Phase 2 |
| I8 | All public constructor/CLI behavior preserved unless a phase explicitly says otherwise (callers: `cli/console_playback.py:539`, `ui/textual_app/app.py:1217`) | — |
| I9 | Every new runtime behavior ships behind an ablation flag following the existing `enable_timer_guard` / `enable_waitable_timer` / `enable_gc_pause` pattern, recorded in `telemetry.record_runtime_options` | `engine.py:324` |

### 0.2 Empirical context the executor MUST internalize (do not re-litigate)

- Player dispatch is already metronomic: **±45 µs** in-game telemetry. Remaining audible hiccups
  were proven game/OS-side, not player bugs.
- **Cold-core hypothesis was tested and REFUTED (2026-06-07):** forcing the core hot
  (min CPU state 100%) and widening spin to 3000 µs produced **no change** in `send_duration`
  tails (p99 916–969 µs across 3 configs; `warm_p99 > cold_p99` consistently). The ~1 ms
  `send_duration` p99 tail lives in the OS/driver input path and is **not player-fixable**.
  ⇒ Do **not** justify any change in this plan by "it will reduce send_duration". The wins here
  are: (a) removing Python overhead between sends, (b) compensating *known median* send latency
  (Phase 4), (c) removing preemption/GIL-induced *outliers* (Phases 2–3), (d) architecture
  cleanliness (Phase 6).
- Real-song corpus: minimum same-key interval is **76 ms** (song `blue`), P50 ≈ 996 ms. The
  schedule-side feasibility floor never binds on real songs.

---

## 1. Current hot path (reference)

```
_run_dispatch loop (dispatch thread, GC paused):
  deadline = coordinator.next_deadline_us(lead)        # min(authored - lead, pending release)
  _wait_until_runtime_deadline(deadline):
      loop: poll commands/focus/render @1ms  → sleep_step ladder (20ms coarse / 1ms ticks / yield)
      final: spin_until_us (spin_threshold_us, default 800)
  _drain_due(now):
      pop_due_pending(now)        → _dispatch_pending_releases → backend.key_up   → SendInput
      pop_due_authored(now+lead)  → _dispatch_down_batch       → backend.key_down → SendInput
                                   (conflict split / expired drop / completion-anchor activate)
  telemetry.record(...) per send
```

Known hot-path costs to remove (measured targets in §8): per-send `sorted()` of a 64-deque
(`engine.py:394`), per-send telemetry dict+string building (`telemetry.py:56`), per-send ctypes
array construction (`inputs.py:304`), triple deduplication (scheduler → backend →
`send_scan_code_batch`), per-call pause arithmetic in `get_elapsed_us` (`engine.py:240`).

---

## 2. Phase plan overview & sequencing

| Phase | Title | Depends on | Risk | Flag |
|---|---|---|---|---|
| 0 | Baseline capture | — | none | — |
| 1 | Hot-path micro-optimizations (no behavior change) | 0 | low | none needed |
| 2 | Dispatch-thread priority ladder (MMCSS et al.) | 0 | low-med | `rt_priority_mode` |
| 3 | GIL switch-interval scope | 0 | low | `enable_switch_interval_tuning` |
| 4 | Adaptive dispatch lead (completion-targeting) | 1 | **med-high** | `enable_adaptive_lead` |
| 5 | Timer wake-error probe + adaptive spin threshold | 1 | med | `enable_adaptive_spin` |
| 6 | Engine decomposition + event-driven wait | 1–5 | high | `enable_event_wait` (for the wait change) |
| 7 | Cleanup, docs, flag graduation | 1–6 | low | — |

Phases 1, 2, 3 are independent of each other and may be separate PRs in any order.
Phase 4 and 5 each require Phase 1 merged (they reuse the send-duration window / probe hooks).
Phase 6 is the big refactor and must come **last among behavior changes** — it re-houses the
mechanisms from 2–5 into a clean structure with all tests already proving them.

Workflow per phase: branch → implement → `uv run pytest` + `uv run ruff check .` +
`uv run pyright` → `uv run python scripts/audit_pipeline_bench.py` → live telemetry protocol (§7)
when the phase touches runtime behavior → submit for acceptance review (§8). Small focused diffs;
no drive-by changes outside the phase's file list.

---

## Phase 0 — Baseline capture

**Goal:** numbers to which every later gate is relative. No code changes (test-only additions allowed).

1. Run and archive (commit output into `docs/perf-baselines/2026-06-baseline.md`):
   - `uv run python scripts/audit_pipeline_bench.py` (default song) and
     `uv run python scripts/audit_pipeline_bench.py "songs/blue.json"` — record per-batch CPU
     p50/p99 and all structural-invariant lines.
   - Full suite timing: `uv run pytest -q` (must be green before starting).
2. Live telemetry baseline (user-assisted, see §7 protocol): 3 runs of `blue` + 1 run of a dense
   song, `local-precise@144` and `balanced@60`, with telemetry CSV enabled. Archive run IDs and
   the summary JSON `lateness_us` / `visible_lateness_us` / `send_duration_us` /
   `idle_gap_us` / `pre_send_spin_us` distributions in the same baseline doc.
3. Add (if missing) a micro-bench section to `scripts/audit_pipeline_bench.py` that isolates:
   `_record_input_path_send_duration` cost, `telemetry.record` cost (enabled), and
   `send_scan_code_batch` struct/array build cost with a mocked `user32.SendInput`. These three
   numbers are the Phase 1 gates.

**Deliverable:** `docs/perf-baselines/2026-06-baseline.md` + any bench additions. No src changes.

---

## Phase 1 — Hot-path micro-optimizations (zero behavior change)

**Goal:** strip fixed Python cost from the inter-send window. Output timelines must be
bit-identical to before (same KeyActions, same dispatch decisions, same CSV schema).

### 1.1 O(1) input-path health estimator — `engine.py:394`
Replace per-send `sorted(self._send_duration_window)` with an incremental threshold counter:
maintain `self._send_over_warn_count` updated on deque append/evict (evict = the element pushed
out when `len == maxlen`; capture it before append). p95 ≤ warn_us ⟺ `over_warn_count ≤
floor(0.05 * len)`. Keep the existing hysteresis (1 s sustained) and the `input_path_warn_us <= 0`
early-out exactly as-is.
*Tests:* new unit test asserting decision-equivalence vs the old sorted-p95 implementation across
randomized sequences (property-style, seeded); existing
`test_input_path_health_flags_sustained_slow_send_duration` must pass unchanged.

### 1.2 Deferred telemetry formatting — `telemetry.py:56`
`record(...)` currently builds a dict + 4 string-joins per event. Change to appending one plain
tuple (or a `slots` dataclass) of raw values; move all joins/dict assembly into `save()` (and into
`get_summary()` if it reads `self.records`). The **CSV schema and summary JSON must stay
byte-identical** (column names, order, formatting).
*Tests:* golden test — run a DryRun playback with telemetry enabled before/after refactor fixture
and diff the CSV header + a known row; all existing telemetry tests
(`test_runtime_dispatch.py::test_telemetry_*`) pass unchanged.

### 1.3 Batch-level INPUT array cache — `platform/win32/inputs.py`
Extend the existing `_INPUT_CACHE` idea one level up: cache the **ctypes array** keyed by
`(scan_codes_tuple, flags)`. Hot path becomes: lookup → single `user32.SendInput(n, cached_array,
sizeof(INPUT))`. Constraints:
- SendInput copies the array by value; cached arrays are never mutated → safe to reuse.
- The partial-send retry path (`sent > 0` slice) must **fall back** to building a fresh array from
  the remaining structs (retry is rare; correctness over speed there).
- Cache is unbounded in theory but bounded in practice (≤15 keys × down/up × observed chord
  combos); add a size guard (e.g. clear above 4096 entries) for hygiene.
*Tests:* unit tests for: identical bytes sent vs old path (mock SendInput capturing arrays),
retry-path slicing correctness with a mock returning partial counts, cache reuse (second call hits
cache), dedupe still applied for non-backend callers.

### 1.4 Dedupe fast path
`WinSendInputBackend._emit` already receives deduplicated, state-filtered tuples
(`_decide_down`/`_decide_up`). Add a private `send_scan_code_batch_trusted(scan_codes, key_up)`
that skips re-dedup (asserts in debug mode instead), used **only** by `_emit`. The public
`send_scan_code_batch` keeps its defensive `dict.fromkeys` for release/retry/panic callers.
*Tests:* assert panic path (`release_all`) still dedupes; backend path sends identical sequences.

### 1.5 Epoch-based elapsed time — `engine.py:240`
`PlaybackState.get_elapsed_us` recomputes pause arithmetic on every call (dozens per event).
Maintain `epoch_us = start_perf + pause_time_us` updated only at pause/resume transitions; when
not paused, `elapsed = clock.now_us() - epoch_us`. Keep the paused-branch math identical.
Preserve the public field names used by tests/telemetry (`pause_time_us`, etc.) or update all
readers in the same commit.
*Tests:* existing pause tests must pass; add unit test for elapsed continuity across
pause→resume→pause sequences (fake clock), asserting equality with the old formula.

**Acceptance gates (Phase 1):** §8 G1–G3; micro-bench: items 1.1–1.3 each show ≥50% cost
reduction on their isolated bench, end-to-end per-batch CPU p50 in `audit_pipeline_bench.py`
reduced ≥20% vs baseline; CSV golden identical; zero test diffs beyond new tests.

---

## Phase 2 — Dispatch-thread priority ladder (MMCSS restored, generalized)

**Goal:** the dispatch thread gets the best scheduling tier the machine offers, chosen from an
ordered ladder, per-thread only (I7). This re-adds and generalizes the `MmcssRegistration` that
was removed in commit `a35e35c`. **Start by recovering the old implementation:**
`git show a35e35c~1:src/sky_music/infrastructure/realtime.py` (class `MmcssRegistration`) and
`git show a35e35c~1:src/sky_music/platform/win32/inputs.py` (avrt bindings:
`AvSetMmThreadCharacteristicsW` / `AvRevertMmThreadCharacteristics`).

### Design
New `infrastructure/rt_priority.py`:

```python
RtPriorityMode = Literal["auto", "mmcss", "time_critical", "highest", "off"]

@dataclass(frozen=True, slots=True)
class RtPriorityOutcome:
    requested_mode: RtPriorityMode
    acquired: str          # e.g. 'mmcss:Pro Audio', 'thread:time_critical', 'off'
    detail: str | None     # failure notes for telemetry/debug

class DispatchThreadPriorityScope:
    """Context manager applied ON the dispatch thread (inside dispatch_target),
    reverted in __exit__ even on exceptions."""
```

Ladder for `auto` (first success wins; each rung fully reverted on exit):
1. **MMCSS**: `AvSetMmThreadCharacteristicsW(name)` trying `("Pro Audio", "Low Latency",
   "Audio", "Games")` strongest-first (empirical: "Low Latency" absent on the user's machine →
   "Pro Audio" wins). On success additionally call `AvSetMmThreadPriority(handle,
   AVRT_PRIORITY_HIGH)` (bind it; ignore failure — registration alone is the main win).
   Revert via `AvRevertMmThreadCharacteristics`.
2. **`SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL)`** (=15). Save the
   previous priority via `GetThreadPriority` and restore it on exit.
3. **`THREAD_PRIORITY_HIGHEST`** (=2), same save/restore.
4. **off** — run unboosted, log it.

Explicit modes pin a single rung (with `off`-fallback on failure, never silently escalating).

### Wiring
- Win32 bindings go in `platform/win32/inputs.py` (lazy `avrt` load consistent with the existing
  win32/non-win32 mock split; `SetThreadPriority`/`GetThreadPriority`/`GetCurrentThread` on
  kernel32). Validate args strictly per AGENTS.md.
- Apply in `engine.py::dispatch_target` (engine.py:1036) — the scope must wrap the same region as
  `high_resolution_timer_scope`, ON the dispatch thread. The non-threaded path
  (`_should_use_dispatch_thread() == False`) does **not** apply priority (it shares the UI
  thread).
- Config: add `rt_priority_mode: str = "auto"` to `AppConfig`; **delete** the dead
  `rt_time_critical` field (read of old key in `config.py:361` maps `true → "auto"`, `false` →
  absent, then is dropped on next save; document in the field comment).
- CLI: `--rt-priority {auto,mmcss,time_critical,highest,off}` override, plumbed like the other
  ablation flags through `RUNTIME_STATE` and both engine construction sites (I8).
- Telemetry: include `RtPriorityOutcome.acquired` in `record_runtime_options`; `debug_log` the
  acquired tier.

### Safety notes (include verbatim in code docstring)
MMCSS task names only select a Windows scheduling profile; they don't require audio work. This is
the OS-sanctioned per-thread mechanism — no process priority class is touched (I7), so the system
and other apps are not starved. It interacts with nothing game-side (I5).

*Tests:* unit tests with a mocked inputs module: ladder order, first-success-wins, rung-failure
fallback, explicit-mode pinning, revert called exactly once on normal exit AND on exception,
`off` produces no Win32 calls; integration: `test_threaded_dispatch.py` ablation-flag test
extended to cover `rt_priority_mode="off"` runs clean.

**Acceptance gates:** §8 G1–G3 + live A/B (§7): with `auto` vs `off`, `lateness_us` p99 and
`idle_gap_us` p99 must not regress; expected (not required) improvement in lateness outliers when
the system is loaded. Telemetry must show the acquired tier on the user's machine
(`mmcss:Pro Audio` expected).

---

## Phase 3 — GIL switch-interval scope

**Goal:** cap GIL handoff latency while the Textual dashboard renders in parallel with dispatch
(the accepted live-dashboard design). Default CPython switch interval is 5 ms — a UI thread mid-
bytecode can deny the spinning dispatch thread the GIL for up to ~5 ms.

- In `RealtimeProcessScope.__enter__` (infrastructure/realtime.py): save
  `sys.getswitchinterval()`, set `sys.setswitchinterval(0.001)`; restore in `__exit__`.
  Constant lives in `realtime.py` as `DISPATCH_SWITCH_INTERVAL_S = 0.001` with a comment
  explaining the dashboard-parallelism rationale.
- Ablation flag `enable_switch_interval_tuning: bool = True` following the existing pattern
  (engine kwarg + `RUNTIME_STATE` + both construction sites + `record_runtime_options`).
- Note in docstring: ctypes `WinDLL` calls release the GIL during the foreign call, so SendInput
  itself never blocks the UI and vice versa; this knob only shortens *bytecode-vs-bytecode*
  handoff.

*Tests:* scope save/restore (including exception path, and nested-scope idempotence); ablation
flag respected. A deterministic GIL-contention test is not required (flaky); rely on live A/B.

**Acceptance gates:** §8 G1–G3; live A/B with the live dashboard active: `lateness_us` p99 during
runs with dashboard not worse than baseline, ideally tail shrink. No UI jank regression reported
by the user (subjective check during §7 protocol).

---

## Phase 4 — Adaptive dispatch lead (completion-targeting) ⚠ highest semantic risk

**Goal:** onsets currently land at `scheduled + send_duration` because the engine spins to the
deadline and *then* calls SendInput. Make the **SendInput completion** land on `scheduled_us` by
leading the call by the predicted send latency. This replaces guessing `--dispatch-lead-us` by
hand and fixes its current asymmetry (lead applies to downs only → systematically stretched
holds).

### Semantics decision (locked here)
**Onset = dispatch completion.** This matches the completion-anchor visibility contract (I1) and
telemetry's `visible_lateness_us`. After this phase, `visible_lateness_us` (not `lateness_us`) is
the primary "are we on time" metric, and `lateness_us` may legitimately be negative — update the
`ExecutionResult` comment (`engine.py:218`) accordingly.

### Design
- New `SendLatencyEstimator` (engine-owned, slots class): per-kind (down/up) EMA of
  `send_duration_us`, α ≈ 0.2, seeded lazily — first N=5 sends of each kind use lead 0 (cold
  estimates are worse than nothing; the schedule's first events are also the warm-up).
- `lead_us(kind) = clamp(round(ema), 0, MAX_LEAD_US)` with `MAX_LEAD_US = 2_000` (≪ frame at
  144 fps, ≪ 76 ms corpus same-key floor).
- **Downs:** `pop_due_authored(now + lead_down)` as today, **but with a no-early-conflict guard**:
  a down batch may NOT be popped early (before its authored `scheduled_us`) if any of its scan
  codes is currently in `active_by_scan_code` or has a pending release. Reason: popping early
  while the same key's release hasn't fired would turn lead into a `dropped_conflict` = lost note,
  violating the prime directive. Implement inside `RuntimeDispatchCoordinator.pop_due_authored`
  (it has the state) — early pop only when all scan codes are idle; otherwise that batch waits for
  its un-led time.
- **Ups (symmetry):** pending releases become due at
  `max(scheduled_release_us - lead_up, release_not_before_us)`. The floor term is **unchanged and
  always wins** — leading the up can never violate I1 because `release_not_before` is anchored to
  down completion. Implement in `PendingRelease.effective_release_us` consumers (pass lead to
  `pop_due_pending` / `next_pending_release_us`).
- Manual `--dispatch-lead-us > 0` disables the estimator and behaves as a fixed symmetric lead
  (same guard rules). New flag `enable_adaptive_lead` (default **False** initially; graduates to
  True in Phase 7 after in-game validation).
- Telemetry: add column `applied_lead_us` per record (0 when disabled). CSV schema change is
  allowed here ONLY as an append-at-end column; update summary JSON with
  `visible_lateness_us` percentiles if not already present.

### Tests (all deterministic, FakeClock + TimedBackend with programmable send durations)
1. With fixed fake send duration D and warm estimator, dispatch completion lands within ±1 tick of
   `scheduled_us` for downs and ups.
2. **Floor invariance under lead:** extend
   `test_observed_hold_never_below_one_frame_under_asymmetric_send_latency` with adaptive lead
   enabled and adversarial asymmetric durations — observed completion-to-completion hold never
   < `min_hold_us`.
3. **No-early-conflict guard:** craft a same-key repeat at exactly the floor; assert the second
   down is never popped before the first release completes and nothing is dropped
   (extend `test_scheduler_feasible_repeat_is_runtime_feasible_invariant`).
4. Cold start: first N sends have `applied_lead_us == 0`.
5. Estimator clamp: pathological 50 ms fake send duration ⇒ lead capped at `MAX_LEAD_US`.
6. Manual lead still works and the existing
   `tests/test_unified_real_path_smoke.py` / lead-propagation test pass.

**Acceptance gates:** §8 G1–G3; live A/B (§7): median `visible_lateness_us` |·| ≤ 300 µs (vs
≈ send-duration median at baseline); `dropped_conflict + dropped_backend + dropped_expired == 0`
on the corpus runs; **mandatory in-game user validation** — user plays `blue` and one dense song
at `local-precise@144` and confirms zero missed notes before this flag may default on.

---

## Phase 5 — Timer wake-error probe + adaptive spin threshold + timer-aware ladder

**Goal:** spin exactly as long as this machine's timer inaccuracy requires — no more (wasted CPU,
GIL hogging), no less (late events when the waitable timer wakes coarse). Reminder from §0.2: do
NOT sell this as a send_duration fix; it is a wakeup-precision and efficiency fix.

### 5.1 Wake-error probe (pre-clock)
In the dispatch thread setup, **before `start_perf` is captured** (same rule as `gc.collect()` —
see `engine.py:1166` comment block; nothing may run after the perf anchor), run ~10 probe sleeps
of 2 ms against the actual sleeper that will be used (waitable timer or RealSleeper fallback) and
record `wake_error = actual - requested`. Derive
`effective_spin_threshold_us = clamp(p_max(wake_errors) + 200, 300, 3_000)`.
- Flag `enable_adaptive_spin` (default False until Phase 7); when off, profile
  `spin_threshold_us` is used as today.
- Record probe stats + chosen threshold in `record_runtime_options`.

### 5.2 Timer-aware sleep ladder — `infrastructure/timing.py`
`PreciseSleeper.sleep_step_towards_us` assumes 1 ms `time.sleep` granularity (5 ms safety buffer,
1 ms tick chain). When the active sleeper is the high-resolution waitable timer, replace the
ladder with: single sleep to `target - guard` where `guard = effective_spin_threshold_us`
(from 5.1), then spin. The 1 ms tick chain exists only to honor the ~1 ms command-poll cadence —
preserve responsiveness by capping any single sleep at the runtime poll interval **only while
commands can arrive** (i.e. keep `min(sleep, 1ms)` behavior until Phase 6's event-driven wait
removes the need). Net effect in Phase 5: the coarse phase shrinks its 5 ms buffer to `guard`,
ticks remain for polling. Move magic numbers (20_000, 5_000, 0.001) into `SleepPolicy` fields with
the current values as defaults.

*Tests:* fake-clock unit tests for the new ladder math (never sleeps past `target - guard`; falls
back to old ladder when flag off or RealSleeper active); probe unit test with a fake sleeper of
known error distribution produces the expected threshold; ordering test asserting probes complete
before `start_perf` (extend `test_runtime_compilation_happens_before_playback_clock_starts`).

**Acceptance gates:** §8 G1–G3; live A/B: `lateness_us` p99 not worse, count of `is_late` events
not worse, measured wakeups-per-second (add an observe-only counter to runtime options) reduced
vs baseline; `pre_send_spin_us` distribution centers near the chosen threshold.

---

## Phase 6 — Engine decomposition + event-driven wait (the big refactor)

**Goal:** `engine.py` (~1,220 lines) is a god class mixing the RT loop, command handling, focus
caching, progress publishing, telemetry plumbing, and thread management. Decompose it without
changing observable behavior, and replace polling waits with kernel-event waits.

### 6.1 Target structure

```
orchestration/
  engine.py               → PlaybackEngine façade ONLY (public ctor/API unchanged, I8;
                             builds the pieces below, owns play())
  dispatch_loop.py        → DispatchLoop: wait → drain → execute. Owns coordinator interaction,
                             ExecutionResult production. No UI, no thread mgmt, no Win32 focus.
  playback_supervisor.py  → thread lifecycle, command routing (queue+event), focus polling,
                             progress consumption (today's _run_threaded_dispatch body).
infrastructure/
  wait_strategy.py        → WaitStrategy protocol + HybridWaitStrategy (absorbs PreciseSleeper
                             ladder, spin, waitable timer, Phase-5 adaptive guard, Phase-6 event
                             wait). PreciseSleeper stays as a thin deprecated alias until Phase 7.
  realtime.py             → RealtimeProcessScope (GC+switchinterval) + DispatchThreadPriorityScope
                             composition helper: enter_realtime_dispatch_context().
```

Specific debts to fix during the move:
- **Kill the `self.sleeper` swap from the dispatch thread** (`engine.py:1039–1070`): the resolved
  sleeper/wait-strategy is constructed per-run and passed as a local into `DispatchLoop`; no
  shared-attribute mutation across threads.
- **`telemetry.record(result: ExecutionResult, …)`**: collapse the ~10 duplicated kwargs
  (`engine.py:608`) into passing the result object + the few extras (generation_ids, outcome).
- Focus cache, `_backend_health_snapshot`, `_record_input_path_send_duration` move to a small
  `DispatchHealthMonitor` owned by DispatchLoop.
- `LoopState`/`PlaybackState` stay; `PlaybackState` gains the Phase-1 epoch field officially.

### 6.2 Event-driven wait (flag `enable_event_wait`, default False until validated)
- New Win32 primitives in `inputs.py`: `create_auto_reset_event()`, `set_event(handle)`,
  `wait_for_multiple_objects((timer, event), timeout_ms)` (strict arg validation; mock-safe on
  non-win32).
- `QueueCommandSource` gains an optional event handle: the supervisor (UI thread) does
  `queue.put(cmd); set_event(h)`. `SharedFocusSignal.set_active` signals the same event on
  *transitions* only.
- `HybridWaitStrategy.wait_until(target_us)`: coarse phase = `WaitForMultipleObjects([timer,
  command_event], …)` sleeping straight to `target - guard`; wake reasons: deadline → spin+send;
  command/focus → drain control work, recompute, re-wait. **Zero periodic polling** in the
  threaded path. The 33 ms progress publish remains, driven by wake opportunities or a dedicated
  supervisor timer (supervisor-side preferred — publishing moves off the dispatch thread's
  responsibilities entirely; DispatchLoop only `update_counters`).
- Non-threaded path (DryRun/tests/`use_dispatch_thread=False`) keeps the polled ladder unchanged.

### 6.3 Behavior-preservation harness (build FIRST, before moving code)
Add `tests/test_engine_equivalence.py`: end-to-end DryRun + FakeClock playback of (a) a golden
synthetic song covering chords, same-key repeats at/above floor, deferred releases, conflicts,
late bursts, pause/resume; record the full `(kind, scan_codes, at_us→actual_us)` timeline and
generation status counts. Capture the pre-refactor output as a committed golden file; the
decomposed engine must reproduce it exactly (flags off) in CI.

*Tests:* the entire existing suite passes with at most import-path updates; thread census still
clean (`test_thread_census.py`); architecture guard test: `dispatch_loop.py` imports no
`sky_music.ui.*` and no `focus` module; event-wait unit tests with mocked primitives (command
wakes the wait, deadline fires on time, event leakage/closure on exit); threaded integration tests
(`test_threaded_dispatch.py`) extended: command-response latency with event wait ≤ polled path.

**Acceptance gates:** §8 G1–G3; equivalence golden byte-identical with all new flags off; live
A/B with `enable_event_wait` on: wakeups/s reduced materially, command (pause/panic) response
p99 ≤ 5 ms, lateness stats not worse; in-game user validation (one full song, pause/resume/panic
exercised).

---

## Phase 7 — Cleanup & graduation

1. Flag graduation (each requires its phase's live validation signed off by the reviewer):
   `enable_adaptive_lead → True`, `enable_adaptive_spin → True`, `enable_event_wait → True`
   defaults; keep all flags available as kill switches.
2. Delete: dead `rt_time_critical` remnants, `PreciseSleeper` alias (update bench/tests),
   any legacy `--dispatch-lead-us` docs implying manual tuning is the primary path (it remains as
   an expert override).
3. Docs: write `docs/rt-dispatch-architecture.md` (post-refactor structure, wait strategy state
   machine, priority ladder, adaptive lead semantics: **onset = dispatch completion**); update
   `docs/timing-principles.md` §3 with the lead-vs-floor interaction (floor always wins); archive
   this plan to `docs/archive/2026-06_rt-pipeline-extreme-optimization-plan.md` with outcome
   stamps per phase.
4. Update `docs/perf-baselines/` with the final numbers next to the Phase-0 baseline.

---

## 7. Live telemetry A/B protocol (used by Phases 0, 2, 3, 4, 5, 6)

Operator: the user (real game required). Executor prepares exact commands; reviewer compares.

1. Songs: `blue` (corpus same-key floor case) + one dense chord song (pick the corpus song with
   highest actions/min; record which).
2. Matrix: `local-precise@144` and `balanced@60`; live dashboard ON for Phase 3/6 runs.
3. 3 runs per cell per arm (feature off = control, feature on = treatment), telemetry CSV enabled.
   Record run IDs in the phase PR description.
4. Compare with `tests/analyze_send_warmup.py` (already supports multi-CSV A/B) + summary JSONs:
   `lateness_us` {p50,p99,max,over_2ms/5ms/10ms}, `visible_lateness_us` {p50 median bias},
   `send_duration_us` {p50,p99} (expected unchanged — see §0.2), `idle_gap_us`,
   `pre_send_spin_us`, generation status counts (all `dropped_* == 0`, `released == downs`).
5. In-game ear/eye check for Phases 4 & 6: user confirms no missed notes, no audible stutter
   regression. **User confirmation is the final word (Evidence Hierarchy rule 1).**

## 8. Acceptance gates (reviewer checklist, applied per phase)

- **G1 — Hygiene:** `uv run pytest` green; `uv run ruff check .` clean; `uv run pyright` clean;
  diff touches only the phase's file list; type hints + frozen/slots dataclasses per AGENTS.md;
  no new dependencies (everything here is ctypes/stdlib).
- **G2 — Structural invariants:** `audit_pipeline_bench.py` reports zero cumulative drift and
  IOI preserved; invariant tests I1–I4 (named in §0.1) pass; new behavior is flag-gated (I9) and
  recorded in `record_runtime_options`.
- **G3 — Performance non-regression:** bench per-batch CPU p50/p99 not worse than baseline
  (Phase 1: ≥20% better p50); suite runtime not pathologically slower.
- **G4 — Live A/B** (phases marked): metrics per §7 meet the phase's stated bars.
- **G5 — Prime directive:** reviewer re-runs the floor tests
  (`test_observed_hold_never_below_one_frame_under_asymmetric_send_latency`,
  `test_hold_floor_preserved_when_thread_stalls_during_hold`, Phase-4 additions) and confirms no
  code path can produce an observed hold < 1 frame. In-game user validation where mandated.

## 9. Explicitly out of scope / forbidden

- Any change to scheduler feasibility math, the ceil one-frame model, or built-in profile values.
- Process priority class, power-plan forcing, CPU-affinity pinning (cold-core refuted, §0.2).
- PostMessage or any non-SendInput injection (probed before; rejected; I5).
- Free-threaded CPython migration (research-only; separate investigation if ever).
- UI/picker/dashboard changes beyond the supervisor seam in Phase 6.
- "While I'm here" refactors outside the phase file lists.
