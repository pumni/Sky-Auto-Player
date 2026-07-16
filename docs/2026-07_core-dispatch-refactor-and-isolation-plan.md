# Core Dispatch Refactor & Isolation Plan

> **Status:** In progress — Phase 0 baseline landed
> **Last updated:** 2026-07-16
> **Baseline commit:** `64e0873` — all `file:line` references in this document are pinned to that commit. If lines have drifted, locate anchors by the quoted code/comment text, never by line number alone.
> **Origin:** Deep review of the scheduling → dispatch → SendInput core (2026-07-16). Findings A1–A6 (confirmed defects), B (accuracy assessment), C (CPU floors), D (isolation gaps + Rust-plan inconsistencies).
> **Audience:** AI coding agents executing the refactor phase by phase. Each phase is independently shippable and independently revertible.

**Sections:** [§0 Scope & Non-Goals](#0-scope--non-goals) · [§1 Invariants](#1-invariants-must-survive-every-phase) · [§2 Execution Rules](#2-execution-rules-for-ai-agents) · [§3 Phase 0 Baseline](#3-phase-0--baseline--regression-harness-05-day) · [§4 Phase 1 Correctness](#4-phase-1--correctness-fixes-1-day) · [§5 Phase 2 Hot-Path Hygiene](#5-phase-2--hot-path-hygiene-1-day) · [§6 Phase 3 CPU Floors](#6-phase-3--cpu-floor-reductions-05-day) · [§7 Phase 4 Structural Isolation](#7-phase-4--structural-isolation-of-the-core-2-days) · [§8 Phase 5 Rust-Plan Alignment](#8-phase-5--rust-migration-plan-alignment-05-day) · [§9 Phase 6 Docs & Final Validation](#9-phase-6--docs--final-validation-05-day) · [§10 Risk Register](#10-risk-register) · [§11 Finding→Phase Traceability](#11-finding--phase-traceability)

---

## 0. Scope & Non-Goals

### 0.1 Scope

Refactor the real-time playback core — `orchestration/dispatch_loop.py`, `orchestration/runtime_dispatch.py`, `orchestration/playback_supervisor.py`, `orchestration/engine.py`, `orchestration/telemetry.py`, `infrastructure/backend.py`, `infrastructure/timing.py`, `infrastructure/wait_strategy.py`, `platform/win32/inputs.py` — to:

1. **Fix confirmed correctness defects** (dead Phase-2 focus gate, pause double-count, clock-injection break).
2. **Remove jitter and CPU waste from the dispatch thread** (mid-play CSV flush, per-send process-name syscalls, ratchet-only adaptive spin).
3. **Make the core boundary structural instead of conventional** so the Rust port (see `rust-migration-plan.md`) replaces a well-defined seam instead of a web of duck-typed side channels.
4. **Fix the Rust migration plan itself** where it contradicts shipped behavior (emit retry policy).

### 0.2 Non-Goals

- **No Rust code in this plan.** This plan prepares the seam; `rust-migration-plan.md` consumes it.
- **No scheduler algorithm changes.** `domain/scheduler.py` (`build_key_actions`, `apply_chord_stagger`, hold planning) is correct, pure, and stays byte-identical except where Phase 0 adds golden coverage.
- **No timing-policy/profile changes.** `min_hold` model, completion anchor, and profile values are out of scope (governed by `timing-principles.md`).
- **No UI/Textual changes** beyond what compiling against renamed types requires.
- **No new runtime dependencies.**

---

## 1. Invariants (must survive every phase)

These are the load-bearing design decisions identified in the review. Any phase that regresses one of them is a failed phase regardless of green gates.

| ID | Invariant | Where it lives today |
|---|---|---|
| I1 | **P0 Security:** SendInput only; no game-memory access, hooks, or injection. | `AGENTS.md` `<SECURITY_MANDATES>` |
| I2 | **Completion anchor:** `release_not_before_us = down_dispatch_completed_us + min_hold_us`. Never re-anchor to dispatch start. | `runtime_dispatch.py:342` + `timing-principles.md` §7 |
| I3 | **Musical no-retry:** a partial note-on SendInput is NEVER completed by a second call; unsent keys are dropped (`DROPPED_BACKEND`). Note-off/panic ALWAYS completes the remainder. | `inputs.py:_send_scan_code_batch_impl` (`complete_remainder`) |
| I4 | **No-early-conflict guard:** a down batch must not be popped before its authored time while any of its scan codes is active/pending (`_early_pop_blocked`). | `runtime_dispatch.py:160-193` |
| I5 | **Single sender:** exactly one thread calls `key_down`/`key_up` during playback. Backend tracking sets are dispatch-thread-owned. | `dispatch_loop.py` finally-block comments, `playback_supervisor.py:444-449` |
| I6 | **Adaptive lead semantics:** lead = EMA of pure send duration (bucketed by polyphony) + positive-only residual bias, clamped to `max_lead_us`; `dispatch_lead_us > 0` overrides the estimator entirely. | `engine.py:SendLatencyEstimator`, `dispatch_loop.py:_down_lead_for_batch` |
| I7 | **Watchdog & release-all safety ladder** (3-pass release, GetAsyncKeyState verification, full-15 panic KEYUP) stays functionally identical. | `backend.py:release_all`, `release_all_full_instrument`, `watchdog.py` |
| I8 | **No process-priority-class changes; per-thread MMCSS/priority only, always reverted.** | `rt_priority.py` |
| I9 | Existing CLI flags, config keys, and telemetry summary JSON keys keep their names and meaning. New keys may be added; none removed without a deprecation note in the summary. | `config.py`, `telemetry.py` |

---

## 2. Execution Rules (for AI agents)

1. **One phase per branch/PR.** Do not start phase N+1 until phase N's gate AND behavioral exit criterion pass. If a phase fails, re-plan the phase; do not paper over with the next phase.
2. **Bug fixes start with a failing test** (AGENTS.md "Goal-driven verification"). Every defect below specifies the test that must fail before the fix and pass after.
3. **Validation altitude per change** (from AGENTS.md): lint `uv run ruff check .` · types `uv run pyright` · tests `uv run pytest`. Full gate for every phase: `uv run ruff check . && uv run pyright && uv run pytest`.
4. **Line references are anchors, not truth.** Verify each quoted snippet exists before editing; if it moved, search for the quoted text.
5. **Do not refactor adjacent working code** beyond what the phase specifies. Keep diffs reviewable.
6. **Free-threaded discipline:** the project runs `3.14+freethreaded`. Every shared-state change in this plan must state its ownership model in a code comment (single-writer, lock, or immutable snapshot). No new bare cross-thread mutable state.
7. When a phase touches `docs/`, update `docs/INDEX.md` in the same phase.

---

## 3. Phase 0 — Baseline & Regression Harness (0.5 day)

**Purpose:** freeze current observable behavior so Phases 1–5 can prove "no unintended change". Nothing in this phase changes production code.

### 3.1 Actions

1. **Golden dispatch timeline test (new): `tests/test_golden_dispatch_timeline.py`.**
   Using `FakeClock` + `DryRunBackend` + direct (non-threaded) mode, play 3 synthetic schedules (single-note melody · chords with same-key repeats · chord-stagger enabled) and snapshot the full backend call history `(kind, scan_codes, started_us)` plus final `generation_status_counts()` into `tests/golden_schedules/dispatch_timeline_v1.json`. Assert exact equality. This is the primary "behavior didn't move" instrument for every later phase.
   - If `tests/golden_schedules/` already contains equivalent coverage, extend rather than duplicate — check first.
2. **Telemetry summary schema snapshot (new): `tests/test_telemetry_summary_schema.py`.**
   Run one telemetry-enabled fake playback; assert the summary JSON key set (recursive key paths, not values) matches a checked-in list. Guards I9.
3. **CPU/wake baseline (manual, recorded not gated):** run `scripts/measure_dispatch_tail.py` (if present; otherwise note its absence in the phase report) and record numbers in `docs/perf-baselines/2026-07-refactor-baseline.md`: spin time per event, supervisor wake rate, dispatch tail latency for the longest song in `songs/`. These are compare-points for Phase 3, not hard gates.

### 3.2 Gate

```powershell
uv run ruff check . && uv run pyright && uv run pytest
```

### 3.3 Behavioral exit criterion

Golden timeline test passes twice in a row (determinism), and the golden JSON is committed. Baseline doc exists with at least the wake-rate numbers or an explicit "tool unavailable" note.

---

## 4. Phase 1 — Correctness Fixes (1 day)

### 4.1 Fix A1: dead Phase-2 pre-down focus gate

**Defect.** `DispatchLoop.run()` contains a duplicated initialization block from a bad merge:

- `dispatch_loop.py:1130` — `self._runtime_focus_signal = focus_signal` (correct).
- `dispatch_loop.py:1150-1162` — a second, duplicated block that re-runs `self._first_down_dispatched = False` and then executes `self._runtime_focus_signal: FocusSignal | None = None`, **overwriting the signal with `None`** before the `try:`.

Consequence: the gate condition at `dispatch_loop.py:561` (`self._runtime_focus_signal is not None`) is always false — the entire Phase-2 check-vs-send race protection (from `2026-07_sendinput-lifecycle-and-timestamp-fidelity-plan.md`) is inert in production.

**Why the existing test doesn't catch it.** `tests/test_focus_input_lifecycle.py:414` (`test_pre_down_focus_gate_blocks_after_first_down`) passes via the *polled* focus gate: `_process_wait_states` pauses the timeline on focus loss, so down #2 naturally dispatches after wall-time 100 000 and the `started_us < 100_000` assertion holds; the `focus_lost` abort tally also comes from the polled gate. The test cannot distinguish the two gates.

**Fix.**

1. Delete the duplicated block at `dispatch_loop.py:1150-1162` (the second `final_abort_reason` comment run, the second `self._first_down_dispatched = False`, and the `self._runtime_focus_signal: FocusSignal | None = None` annotated assignment). Keep the assignments at `:1130-1132`. Move the attribute's *declaration* (with its long ownership comment) to `__init__` as `self._runtime_focus_signal: FocusSignal | None = None` so the type annotation lives where declarations belong.
2. **Discriminating test first (must fail on baseline).** Rewrite/extend the Phase-2 test so only the pre-down gate can satisfy it:
   - Assert a telemetry record with `runtime_outcome == "blocked_unfocused"` exists for the gated down (the polled gate never produces that outcome — it produces a pause + `focus_lost` abort only).
   - Keep the existing polled-gate assertions in a separate test so both mechanisms have independent coverage.
3. Add a micro-test asserting `loop._runtime_focus_signal is focus_signal` immediately observable via behavior: e.g. a `FocusSignal` stub whose `is_active` is called at least once from `_dispatch_down_batch` during a run with ≥2 down batches and `require_focus=True`.

### 4.2 Fix A2: pause accounting double-count (single-owner pause state machine)

**Defect.** `PlaybackState` (dispatch_loop.py:81-118) keeps two independent anchors (`manual_pause_started_us`, `focus_pause_started_us`) and two independent accumulation sites (`_handle_commands` unpause at `:816-819`; focus regain at `:898-900`). Interleaving *focus-lost → user pause → user unpause → focus regain* counts the overlap twice → `epoch_us` over-advances → the rest of the song is delayed by the overlap. Additionally, `get_elapsed_us` with both anchors set returns a value that **decreases with wall time** (`elapsed -= now - focus_pause_started`), clamped at 0 — the progress display runs backwards.

**Fix — replace dual anchors with one pause interval owner:**

1. New model inside `PlaybackState`:
   ```python
   pause_reasons: set[str]          # subset of {"manual", "focus"} — nonempty ⇒ paused
   pause_interval_started_us: int | None  # wall anchor of the CURRENT contiguous paused interval
   ```
   - Entering pause (either reason) when `pause_reasons` was empty: set the anchor.
   - Adding a second reason while paused: add to the set only — anchor unchanged.
   - Removing a reason: if the set becomes empty, accumulate `now - pause_interval_started_us` into `pause_time_us` exactly once, clear the anchor.
   - `get_elapsed_us`: if paused, return `pause_interval_started_us - epoch_us` (frozen, never decreasing); else `now - epoch_us`. Keep the `max(0, …)` clamp.
2. **Per-reason telemetry attribution:** `telemetry.record_pause(reason, duration)` currently receives one duration per reason. Preserve the summary schema (I9) by attributing each contiguous interval to the reason(s) active during it: record the full interval duration under the *first* reason that opened it, and record `0`-duration entries are NOT needed — document the attribution rule in a comment. (The previous behavior double-attributed overlap; the summary consumers only aggregate per-reason totals, verified by the Phase 0 schema snapshot.)
3. Keep the public method surface (`is_paused()`, `update_pause_time()`, `rebase_epoch()`, `get_elapsed_us()`) so `_wait_until_runtime_deadline`, the supervisor, and tests keep compiling. `manual_pause_started_us` / `focus_pause_started_us` have external readers (grep before removing — `_handle_commands`, `_process_wait_states`, supervisor `state.is_paused()`, tests). Provide thin compatibility properties **only if** grep shows out-of-file readers; otherwise migrate the readers in this phase.
4. **Failing tests first (new `tests/test_pause_state_machine.py`):**
   - `test_focus_then_manual_pause_no_double_count`: fake clock; lose focus at t=10 000; manual pause at t=20 000; manual unpause at t=50 000; focus regain at t=80 000. Assert `pause_time_us == 70_000` (one contiguous interval), not `90_000`.
   - `test_elapsed_frozen_while_double_paused`: with both reasons active, `get_elapsed_us` is constant as the fake clock advances.
   - `test_single_reason_roundtrip_unchanged`: plain manual pause/unpause and plain focus lose/regain produce identical `pause_time_us` to the old model (regression guard).

**Design note for the Rust port:** this state machine is the shape §5 of `rust-migration-plan.md` should adopt (single `pause_accumulated_us` + one anchor + reason set), replacing its current copy of the dual-anchor fields.

### 4.3 Fix A6a: clock injection break in `WinSendInputBackend._emit`

**Defect.** `backend.py:426` hardcodes `time.perf_counter_ns() // 1000` for `send_completed_us` while the loop's timeline uses the injected `Clock`. With any non-`PerfCounterClock` clock the two time bases diverge silently; today this is only avoided by convention (`DryRunBackend` returns `None`).

**Fix.** Give `WinSendInputBackend.__init__` an optional `clock: Clock | None = None` parameter; store `self._now_us = clock.now_us if clock else (lambda: time.perf_counter_ns() // 1000)`. `engine.py` passes its clock when constructing… **check construction sites first**: backends are constructed by callers (`console_playback.py`, `app.py`) before the engine exists. Therefore instead: add `set_clock(clock: Clock)` to `WinSendInputBackend` and call it from `PlaybackEngine.__init__` when `backend` is a `WinSendInputBackend` (duck-check via `getattr(backend, "set_clock", None)` is NOT acceptable in this plan — Phase 4 formalizes the protocol; here, add `set_clock` to the `InputBackend` Protocol with a default-no-op implementation on `_TrackedKeyState`).
Test: fake clock + `WinSendInputBackend` with mocked `inputs_module` — assert `send_completed_us` is on the fake-clock axis.

### 4.4 Fix A6b (micro): estimator contamination by no-op sends

`dispatch_loop.py:637` feeds `result.send_duration_pure_us` into the estimator even when the backend sent nothing (all keys were duplicates → `sent == ()`, duration ≈ 0), dragging the down-lead EMA toward 0. Guard: skip `estimator.update(DOWN, …)` when `result.sent_scan_codes` is empty. Same guard for the UP path (`:687`, `:745`) when `sent_scan_codes` is empty. Add a unit test on `SendLatencyEstimator` + a loop-level test with a duplicate-down schedule asserting the lead is unchanged after the no-op send.

### 4.5 Gate

```powershell
uv run ruff check . && uv run pyright && uv run pytest
```

### 4.6 Behavioral exit criteria

- New discriminating gate test fails on baseline commit, passes after; `blocked_unfocused` appears in telemetry for the gated down.
- `test_focus_then_manual_pause_no_double_count` fails on baseline, passes after.
- Phase 0 golden timeline byte-identical (no timing change for non-pause, focused playback).

---

## 5. Phase 2 — Hot-Path Hygiene (1 day)

Everything in this phase removes work from the dispatch thread between SendInput calls. None of it may change the dispatch *timeline* (golden test is the referee).

### 5.1 Fix A3: telemetry CSV flush off the RT thread

**Defect.** `telemetry.py:365-368`: when enabled, `record()` (called synchronously from `_execute_action`) triggers `_flush_records_to_csv()` at 10 000 records — synchronous `csv.writerows` + `file.flush()` on the dispatch thread mid-song. Calibration/benchmark sessions (exactly when telemetry is on) self-inject multi-ms stalls.

**Fix (choose the simple option, not a new thread):**

1. Remove the flush trigger from `record()`. Replace with flush points that are already off the hot window: (a) inside `record_pause` (playback is paused), (b) in `TelemetryLogger.save()` (already flushes), and (c) a new `flush_if_large()` method called by `DispatchLoop` **only** from `_process_wait_states`' paused/wait branches (the loop is idle-polling there).
2. Memory bound concern: without mid-play flush, `records` grows unbounded for very long songs. Bound it: raise `_TELEMETRY_FLUSH_CHUNK` usage into `flush_if_large()` with the same 10 000 threshold, and ADD a hard cap `_TELEMETRY_MAX_BUFFER = 200_000` at which `record()` flushes synchronously anyway (explicitly documented as the pathological-fallback: a >100k-event song with zero pauses accepts one stall rather than unbounded RSS — mirrors the RAM-hygiene plan's philosophy).
3. Test: enabled telemetry, fake playback of 25 000 events with no pause — assert no CSV write occurs before `save()` (patch `_flush_records_to_csv` with a recorder), then assert `save()` writes all rows. Second test: with a pause mid-song, assert the pause-path flush fires.

### 5.2 Fix A4: per-send focus refresh does OpenProcess

**Defect.** `_execute_action` calls `health_monitor.focus_is_active()` after every send (`dispatch_loop.py:466-472`). On each 2 ms TTL expiry this runs `is_sky_active()` → `is_sky_window_valid()` (`inputs.py:715-739`) → `GetWindowThreadProcessId` + `OpenProcess` + `QueryFullProcessImageNameW` + `CloseHandle`. Up to ~500 process-handle opens/second on the dispatch thread during dense passages, purely to feed the `send_while_unfocused` diagnostic counter.

**Fix.**

1. Add `inputs.is_foreground_cached_hwnd() -> bool`: `sky is not None and user32.GetForegroundWindow() == sky` — **no revalidation, no process lookup**. Document: the process behind a live HWND cannot change; staleness is bounded by the polled gate + supervisor focus sampling which still use the full check.
2. `DispatchHealthMonitor.focus_is_active()` keeps its 2 ms TTL but calls the cheap check. The full `focus_guard.is_active()` remains the authority for: supervisor periodic sampling (`playback_supervisor.py:431`), the polled pause gate, `run()`-entry checks, and `engine.play()` pre-start wait — those cadences are 20–50 ms and human-facing.
3. Under threaded dispatch, prefer reading the runtime `FocusSignal` (already sampled by the supervisor) instead of any syscall: pass the signal into `DispatchHealthMonitor` at `run()` entry (`set_runtime_signal(focus_signal)`), use it when present, cheap-HWND check as direct-mode fallback. This also removes the last blocking focus syscall from the dispatch thread entirely in the default (threaded) configuration.
4. Test: counting stub for `focus_guard.is_active` — during a threaded-mode fake run with N sends, assert zero calls originate from the dispatch thread path (only supervisor cadence calls remain).

### 5.3 Layering: remove per-event platform import from `_execute_action`

`dispatch_loop.py:468-471` does `from sky_music.platform.win32 import inputs` inside the hot function to bump `note_send_while_unfocused()`. Replace with a counter callback injected at loop construction (`unfocused_send_hook: Callable[[], None] | None`), wired by `engine.py` to the platform counter. This is the first step of the Phase 4 boundary work but lands here because it sits inside the hot path.

### 5.4 Gate & exit criteria

```powershell
uv run ruff check . && uv run pyright && uv run pytest
```

- Golden dispatch timeline byte-identical.
- New tests from §5.1–§5.2 fail-before/pass-after.
- Grep gate: `Grep "from sky_music.platform" src/sky_music/orchestration/dispatch_loop.py` returns only the `TYPE_CHECKING`/finally-block occurrences explicitly allowed until Phase 4 (list them in the PR description).

---

## 6. Phase 3 — CPU Floor Reductions (0.5 day)

### 6.1 Fix A5: adaptive-spin reprobe ratchet + contaminated samples

**Defects** (active only with `enable_reprobe`):

- `dispatch_loop.py:1192-1196` applies a recomputed threshold only when *greater* — a transient load spike raises per-event spin (CPU) permanently for the rest of the song.
- `_record_overshoot` is called at `dispatch_loop.py:999-1001` on the "already past deadline" branch, where overshoot reflects the previous drain's duration, not timer wake error — contaminated samples drag the threshold toward the 3 000 µs cap.

**Fix.**

1. Sample only true timer-wake overshoot: keep the call after `spin_until_us` (`:1008-1010`); delete the call on the already-late branch.
2. Symmetric apply: `self.spin_threshold_us = new_threshold` (both directions), still clamped to `[700, 3_000]` by `_recompute_spin_threshold_from_overshoot`. Never below the profile's configured `spin_threshold_us`? — **No:** allow it; the probe measured real wake error. Record each applied change into telemetry `runtime_options["reprobe_applied_thresholds"]` (append list) for observability.
3. Tests: feed synthetic overshoot samples; assert threshold decreases after calm samples follow a spike; assert the already-late branch contributes no samples (unit-test `_record_overshoot` call sites via a fake wait strategy).

### 6.2 `_ARRAY_CACHE` mislabeled LRU

`inputs.py:409-412` + `:522-540`: comment says LRU, but `.get()` never moves entries — eviction is FIFO. At 8 192 capacity with per-song `clear_array_cache()` this is inert; fix the *label* (comment: "bounded FIFO — sufficient because the cache is cleared per song and songs stay far below capacity") rather than adding `move_to_end` to the hot path. One-line change; no test needed beyond ruff.

### 6.3 (Optional, only if baseline shows it matters) Supervisor tick

`playback_supervisor.py:460` sleeps 10 ms fixed → 100 wake/s. Acceptable; do NOT event-drive it in this plan (complexity not justified — the Rust plan removes this loop's role entirely). Record the decision here so future agents don't "optimize" it ad hoc.

### 6.4 Gate & exit criteria

```powershell
uv run ruff check . && uv run pyright && uv run pytest
```

- Golden timeline unchanged.
- With reprobe enabled in a synthetic test: threshold recovers downward after a spike (new test), and `reprobe_applied_thresholds` appears in the summary.

---

## 7. Phase 4 — Structural Isolation of the Core (2 days)

**Goal:** the dispatch core becomes a package with an explicit, typed, minimal surface — the exact seam `RustBridge` will later occupy. After this phase, "the core" = *(compiled schedule in, backend + clock + waiter in, command/focus/progress ports in, result + telemetry stream out)* and nothing else.

### 7.1 Target seam (definition of done)

```
sky_music/orchestration/core/          # NEW package — the future Rust seam
├── __init__.py                        # exports: DispatchCore, CoreConfig, CorePorts, PlaybackCommand
├── loop.py                            # DispatchLoop (moved, trimmed)
├── coordinator.py                     # RuntimeDispatchCoordinator (moved verbatim)
├── state.py                           # PlaybackState (post-Phase-1 state machine)
└── ports.py                           # Protocols: InputBackend, Clock, Sleeper, WaitStrategy,
                                       #   CommandSource, FocusSignal, ProgressSink, TelemetrySink,
                                       #   LeadEstimator
```

Import-linting rule (enforced by test, §7.6): **nothing under `core/` imports `sky_music.platform.*`, `sky_music.ui.*`, or `sky_music.infrastructure.focus`.** Platform access happens only through injected ports.

### 7.2 Kill the duck-typed backend side channels

`InputBackend` Protocol (`backend.py:47-67`) currently under-describes the real contract; `DispatchLoop` compensates with `getattr`:

- `dispatch_loop.py:362` — `getattr(backend, "release_all_full_instrument", None)`
- `dispatch_loop.py:1232` — `getattr(self.backend, "get_send_diagnostics", None)`
- Phase 1 added `set_clock`.

**Fix:** promote all three into the `InputBackend` Protocol. `_TrackedKeyState` provides defaults (`release_all_full_instrument` → `release_all()`; `get_send_diagnostics` → zeros dict, already exists; `set_clock` → no-op). Remove every `getattr`/`callable` probe from the loop. Update test fakes that implement the Protocol structurally (grep `class .*Backend` in `tests/`).

### 7.3 Enum-typed commands

Commands cross three layers as bare strings (`"pause" | "skip" | "quit" | "panic" | "refocus"`). Introduce `PlaybackCommand(StrEnum)` in `core/ports.py`; `CommandSource.poll() -> PlaybackCommand | None`. Keep `StrEnum` so every existing `== "pause"` comparison keeps working during migration, then migrate comparisons in `core/` to enum members. Hotkey/UI producers (`hotkeys.py`, Textual command bridge, `QueueCommandSource` fill sites) construct the enum at the edge. Unknown strings from legacy producers: map via `PlaybackCommand(value)` with `ValueError` → ignore + debug_log (defensive, tested).

### 7.4 Remove back-references from core to engine

- **`probe_callback` weakref** (`engine.py:647-654` + `dispatch_loop.py:888-890`): the loop calls back into the engine, which mutates the loop's `spin_threshold_us`. Replace with a `SpinThresholdProber` port: a small class owning `_measure_spin_threshold` logic (move from engine), injected into the loop; the loop applies the returned value itself. Engine keeps only telemetry recording of probe results via the prober's result object.
- **Estimator ownership:** the estimator object stays engine-owned (it must survive across plays for the warm cache) but the loop must depend only on the `LeadEstimator` protocol (already true — verify and move the Protocol into `core/ports.py`).
- **Shared `PlaybackState` reads from the supervisor thread** (`playback_supervisor.py:419,448`): supervisor publishes progress using `state.get_elapsed_us(self.clock)` while the dispatch thread mutates pause fields. Post-Phase-1 the state is a cleaner machine but still shared. Fix: give `PlaybackState` an `elapsed_snapshot_us()` documented as approximate-read-only for display, implemented over a single atomically-assigned tuple `self._display_snapshot = (epoch_us, pause_anchor, paused)` updated by the dispatch thread on every transition (single-writer; one reference assignment = atomic under free-threading). Supervisor uses only the snapshot method.

### 7.5 Contain `inputs.py` module globals

Full removal is the Rust plan's Phase 6 job. Here, containment only:

1. Move the diagnostics counters (`_PARTIAL_SEND_EVENTS` … `_IMPOSSIBLE_SAME_KEY_REPEATS`) into a `SendDiagnostics` dataclass instance owned by the module (`_DIAG = SendDiagnostics()`), with `reset()`/`snapshot()` methods replacing `reset_send_diagnostics()`/`get_send_diagnostics()` bodies (public function names stay — I9). Single-writer contract documented on the class, not scattered.
2. `sky` HWND global and `EXPECTED_PROCESS_NAMES` mutation from `main.py`: wrap writes in explicit setter functions (`set_expected_process_names`, already-existing `reset_window_cache`) and grep-fix direct assignments. No behavior change.
3. Do NOT touch `_INPUT_CACHE` / `_ARRAY_CACHE` beyond Phase 3's comment fix.

### 7.6 Boundary enforcement test

New `tests/test_core_boundary.py`: walk `sky_music/orchestration/core/*.py` with `ast`, assert no `import`/`from` of forbidden prefixes (`sky_music.platform`, `sky_music.ui`, `sky_music.infrastructure.focus`). Also assert `sky_music.orchestration.core` does not import `engine` (no cycles back out). This test is the structural replacement for today's comment-based contracts.

### 7.7 Compatibility

`engine.py` already re-exports moved names (`DispatchLoop`, `PlaybackState`, etc. — `engine.py:33-75`); extend those re-exports so **zero test-file imports need to change in this phase** (`orchestration/dispatch_loop.py` becomes a thin re-export shim module; deleting it is a later cleanup, noted in §9). Grep gate: `uv run pytest` green without editing test imports proves the shim works.

### 7.8 Gate & exit criteria

```powershell
uv run ruff check . && uv run pyright && uv run pytest
```

- `tests/test_core_boundary.py` passes.
- Golden dispatch timeline byte-identical.
- No `getattr(` against the backend anywhere under `core/` (grep gate).
- `uv run python -m app --selftest-textual` passes (wiring smoke).

---

## 8. Phase 5 — Rust Migration Plan Alignment (0.5 day)

Documentation-only phase; edits `docs/rust-migration-plan.md` so the port cannot silently regress shipped policy.

1. **§8 `emit()` — CRITICAL:** the pseudo-code retries partial sends unconditionally (`remaining = &remaining[sent..]` loop), which applies release semantics to note-on and deletes invariant I3 (musical no-retry). Rewrite the section: `emit(cache, scans, key_up, complete_remainder: bool)`; note-on (`complete_remainder=false`) returns the atomic prefix and never issues a second `SendInput`; note-off/panic completes the remainder. Fix the stray out-of-scope `sent` in `EmitResult.partial_send` and the `sent: scans.to_vec()` bug (must be the actually-landed prefix). Add a parity-test requirement: *partial note-on drops the tail; coordinator marks `DROPPED_BACKEND`; diagnostics count `keys_dropped`* — mirroring `tests` that cover the Python path.
2. **§10 drop policy wording:** "drop the oldest pending batch (LIFO drop)" is self-contradictory. State the intended policy explicitly (drop-oldest = FIFO drop) and why (late tail data more valuable than stale mid-play batches).
3. **§5 `RuntimeKernel` pause fields:** replace the dual-anchor copy (`pause_started_ns`, `focus_pause_started_ns`) with the Phase-1 single-interval state machine (reason set + one anchor + accumulator). Reference this plan §4.2.
4. **Focus ownership:** plan currently duplicates the focus guard in Rust (`focus/guard.rs`, 2 ms TTL) while Python keeps `focus.py` — two sources of truth. Add a decision note: the supervisor-side Python guard owns focus *policy* (pause/resume); the Rust worker consumes a shared flag (equivalent of `SharedFocusSignal`) plus the cheap foreground-HWND compare from Phase 2 §5.2 for the pre-send gate. Delete the Rust-side process-name revalidation from the plan.
5. Update the plan's §0 background numbers to reference the Phase 0 baseline doc.

**Gate:** `uv run ruff check .` (docs lint-neutral) + reviewer read. **Exit criterion:** every I-invariant from §1 of this plan is either cited or explicitly N/A in `rust-migration-plan.md`.

---

## 9. Phase 6 — Docs & Final Validation (0.5 day)

1. Update `docs/rt-dispatch-architecture.md`: new `core/` package layout, ports list, pause state machine, focus-check ownership table (who calls the full check vs the cheap check, at what cadence).
2. Update `docs/INDEX.md`: mark this plan **Implemented** with an as-built divergence note (any place execution deviated from this document — keep the debrief honest).
3. Re-run the Phase 0 CPU baseline; append results to `docs/perf-baselines/2026-07-refactor-baseline.md` (before/after table: dispatch-thread syscalls per send, spin threshold trajectory with reprobe, telemetry-enabled tail latency).
4. Optional cleanup **only if all gates green**: delete the `orchestration/dispatch_loop.py` re-export shim and migrate test imports in one mechanical commit.

**Final gate (full pipeline):**

```powershell
uv run ruff check . && uv run pyright && uv run pytest
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
uv run --env-file .env python -m build_app
```

**Exit criterion:** build smoke test (`--selftest-textual`) passes; golden timeline unchanged since Phase 0; telemetry summary schema snapshot unchanged (plus explicitly-added keys documented).

---

## 10. Risk Register

| # | Risk | Phase | Mitigation |
|---|---|---|---|
| R1 | Fixing A1 activates a gate that was never live in production → new mid-song aborts on marginal focus flapping. | 1 | The gate only fires when `require_focus` and the shared signal reads inactive — same signal the polled gate already pauses on; worst case is an earlier, cleaner drop. Watch `blocked_unfocused` counts in telemetry after release. |
| R2 | Pause state machine changes summary attribution of pause durations. | 1 | Phase 0 schema snapshot + explicit attribution rule; per-reason totals may shift for the (previously double-counted) overlap case only — that is the fix, not a regression. |
| R3 | Removing mid-play telemetry flush grows RSS on pathological songs. | 2 | Hard cap `_TELEMETRY_MAX_BUFFER` with documented synchronous fallback. |
| R4 | Cheap HWND-only focus check misses process replacement behind a reused HWND. | 2 | HWND reuse requires window destruction → `IsWindow` fails on the *full* checks that still run at 20–50 ms cadence; exposure window is bounded and smaller than today's 2 ms-TTL blind spot under load. Documented in code. |
| R5 | Moving files (Phase 4) breaks the many `engine.py` compat importers. | 4 | Shim module + re-exports; gate requires zero test-import edits; deletion deferred to Phase 6 step 4. |
| R6 | Symmetric reprobe lowers spin threshold below real wake error on a machine whose probe ran during an idle moment. | 3 | Threshold recomputes every 5 s of playback from a 200-sample rolling window; a bad low value self-corrects within one window. Clamp floor 700 µs unchanged. |
| R7 | Free-threaded races introduced by new shared objects (`SharedFocusSignal` into health monitor, display snapshot). | 2/4 | Single-writer + single-reference-assignment patterns only; each new shared object documents its ownership model (Execution Rule 6); no new locks on the hot path. |

---

## 11. Finding → Phase Traceability

| Review finding | Severity | Phase | Section |
|---|---|---|---|
| A1 dead pre-down focus gate (`dispatch_loop.py:1162`) + non-discriminating test | High | 1 | §4.1 |
| A2 pause double-count / backwards elapsed | Medium | 1 | §4.2 |
| A3 telemetry CSV flush on RT thread | Medium (when telemetry on) | 2 | §5.1 |
| A4 OpenProcess-per-2ms focus refresh on dispatch thread | Medium | 2 | §5.2 |
| A5 reprobe ratchet-up-only + contaminated overshoot samples | Low-Medium | 3 | §6.1 |
| A6a backend `_emit` hardcodes perf_counter (clock-injection break) | Low | 1 | §4.3 |
| A6b estimator contamination by no-op sends | Low | 1 | §4.4 |
| A6c `_ARRAY_CACHE` FIFO mislabeled LRU | Trivial | 3 | §6.2 |
| D1 direct platform imports in DispatchLoop | Structural | 2+4 | §5.3, §7.1 |
| D2 duck-typed backend side channels | Structural | 4 | §7.2 |
| D3 stringly-typed commands | Structural | 4 | §7.3 |
| D4 core→engine back-references (probe weakref, shared state reads) | Structural | 4 | §7.4 |
| D5 `inputs.py` module-global contracts by convention | Structural | 4 | §7.5 |
| D6 rust-migration-plan §8 emit retry contradicts I3 | High (future) | 5 | §8.1 |
| D7 rust-migration-plan §10 drop-policy contradiction, §5 pause copy, focus duplication | Low (future) | 5 | §8.2–8.4 |
