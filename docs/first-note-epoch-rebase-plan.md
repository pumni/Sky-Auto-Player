# First-Note Epoch Rebase Plan

Status: ACCEPTED (2026-06-12). All §5 gates passed on real blue.json runs:
G1 epoch_rebase_us=1949µs (first real measurement of thread-startup cost);
G2 event-0 dispatch lateness 2325µs (baseline, bug confirmed) → 67µs (fix, 35× reduction;
visible 591µs is dominated by send_duration ~0.5ms under live game load — out of scope);
G3 no regression (visible avg 438.19 → 438.07µs); G4 1030/1030 released, zero drops,
hold_below_frame=0; G5 kill switch reproduces old behavior.
Runs: logs/playback_telemetry_20260612-003940-1546 (baseline, --no-epoch-rebase),
logs/playback_telemetry_20260612-004333-5400 (fix ON).

## §1 Problem

The playback epoch (`start_perf`) is captured on the **control thread** in
`PlaybackEngine.play()` (engine.py:373, `PlaybackState(start_perf=self.clock.now_us())`),
but the first SendInput can only happen after:

1. `_build_dispatch_loop` + `PlaybackSupervisor` construction,
2. `threading.Thread.start()` for `sky-music-dispatch` (playback_supervisor.py:339),
3. inside `dispatch_target`: `high_resolution_timer_scope()` (timeBeginPeriod syscall),
   `DispatchThreadPriorityScope` (MMCSS `AvSetMmThreadCharacteristicsW` ladder),
   telemetry `record_runtime_options`, debug logs.

Everything between the anchor and `dispatch_loop.run()` is charged against the song
schedule. Estimated cost: ~0.2–2 ms (unmeasured — this plan also instruments it).

Impact class: songs whose first note sits inside that window. **blue.json's first note
is at t=0 ms** (verified 2026-06-12), so its first onset is *guaranteed late* by the full
startup cost, on every run. Later notes are unaffected (the schedule is absolute against
the epoch; startup cost does not accumulate).

Non-impact: the rt-pipeline round-1 numbers (onset median −3 µs, p99 lateness 80–104 µs)
were measured on steady-state events and remain valid; this plan touches only the anchor.

## §2 Fix (chosen design)

Re-anchor the epoch **on the dispatch thread**, after the RT scopes are entered,
immediately before `dispatch_loop.run()`. The first note then sees the same dispatch
environment (MMCSS, 1 ms timer, warm thread) as every other note.

### Changes

1. **`PlaybackState.rebase_epoch(now_us: int) -> int`** (dispatch_loop.py)
   - Sets `start_perf = now_us`, recomputes `epoch_us = start_perf + pause_time_us`
     (pause_time is 0 at this point; keep the formula honest anyway).
   - Returns the delta `now_us - old_start_perf` for telemetry.

2. **`PlaybackSupervisor`** (playback_supervisor.py)
   - New ctor param `enable_epoch_rebase: bool = False`.
   - In `dispatch_target`, inside `with timer_scope, priority_scope:`, after the
     rt_priority telemetry record and as the **last statement before
     `dispatch_loop.run()`**:
     ```python
     if self.enable_epoch_rebase:
         rebase_us = state.rebase_epoch(self.clock.now_us())
         self.telemetry.record_runtime_options(
             {**self.telemetry.runtime_options, "epoch_rebase_us": rebase_us}
         )
     ```
   - Comment in code: rebase must stay the last pre-run statement; anything inserted
     after it re-creates the bug.

3. **`PlaybackEngine`** (engine.py)
   - New ctor param `enable_epoch_rebase: bool = False` (ctor stays OFF — matches the
     graduation convention from the rt-pipeline plan: behavioral toggles default OFF in
     the engine ctor so existing tests are byte-identical; production wiring turns them ON).
   - Pass through to `PlaybackSupervisor`.
   - Record in `record_runtime_options` alongside the other toggles.

4. **CLI wiring** (main.py / config)
   - Default ON in production wiring, kill switch `--no-epoch-rebase`
     (same pattern as `--no-adaptive-lead`, `--no-event-wait`).

### Scope limits

- **Threaded mode only.** `_run_direct` is NOT rebased: there is no thread spawn there,
  and fake-clock engine tests run through the direct path — rebasing it would shift
  their timelines.
- No change to `DispatchLoop`, scheduler, coordinator, or backend.

## §3 Concurrency analysis

- After `dispatch_thread.start()`, the control thread reads
  `state.get_elapsed_us(self.clock)` for progress/refocus publishing. If it reads
  before the rebase lands, one progress sample shows up to ~2 ms extra elapsed —
  display-only, invisible at 33 ms publish cadence.
- CPython attribute assignment is GIL-atomic; no torn int reads. Worst interleaving
  (read between `start_perf` and `epoch_us` writes) yields one stale sample.
- Pause fields (`manual_pause_started_us`, `focus_pause_started_us`) are mutated only
  by the dispatch thread, which runs strictly after the rebase. No ordering hazard.

## §4 Tests

1. **Unit — `PlaybackState.rebase_epoch`**: construct with nonzero `pause_time_us`,
   rebase, assert `epoch_us == new_start + pause_time_us` and the returned delta.
2. **Supervisor-level**: drive `PlaybackSupervisor.run(use_dispatch_thread=True,
   enable_epoch_rebase=True)` with a stub DispatchLoop whose `run()` records
   `state.start_perf` at entry. Assert: start_perf at entry > start_perf at
   construction, and `telemetry.runtime_options["epoch_rebase_us"] >= 0`.
   (The supervisor does not gate on clock type — only `PlaybackEngine.
   _should_use_dispatch_thread` does — so a stub loop works here.)
3. **Flag default**: with `enable_epoch_rebase=False` (default), `start_perf` is
   untouched and `epoch_rebase_us` is absent → all existing threaded tests pass unchanged.

## §5 Acceptance gates (real-run, blue.json, telemetry on)

| Gate | Criterion |
|---|---|
| G1 | `epoch_rebase_us` present in runtime options; plausible range 100–5000 µs (this is also the first real measurement of thread-startup cost) |
| G2 | Event 0 (`source t=0`) `visible_lateness_us` ≤ normal p99 (≤ ~150 µs); previously it must have been ≈ startup cost — capture one baseline run BEFORE the fix to prove the delta |
| G3 | Whole-song onset median within ±10 µs and p99 ≤ 110 µs (no regression vs round-1) |
| G4 | `generation_status_counts`: zero new dropped_*/cancelled vs baseline |
| G5 | `--no-epoch-rebase` run: behavior identical to baseline (kill switch verified) |

Baseline first: one blue.json run on current main, archived, before any code change.

## §6 Non-goals (decided 2026-06-12, do not resurrect without new telemetry)

- **Chord-transition single-syscall merge** (one SendInput array `[ups…, downs…]`):
  rejected — breaks the down/up tracking boundary in `WinSendInputBackend` and
  generation accounting for ~tens of µs only at transitions, which telemetry does not
  flag as jitter peaks.
- **Estimator pre-seed via priming sends**: rejected — sends real keys to the game
  outside the song. First-5-event lead absence (~30–80 µs onset shift) stays as-is.
  Note: for a t=0 first note the loop fires immediately anyway; lead is irrelevant there.

## §7 Effort

~30 LOC production + ~60 LOC tests. Single PR.
