# Dispatch/chord timing residual review — 2026-07-23

> **Status:** Review report, not a normative specification.
> Current code and the canonical documents listed in `docs/INDEX.md` win on conflict.
> This report reviews the tree after the 2026-07-23 dispatch hardening work.

## 1. Scope and locked assumptions

This review covers:

- exact-timestamp chord scheduling and optional chord stagger;
- generation compilation, pending releases, deadline selection, and due draining;
- adaptive send-latency estimation and its cross-session cache;
- supervisor/dispatch-thread shutdown and Win32 wait outcomes;
- the final `SendInput` batch/retry policy;
- CPU and RAM costs that are directly attributable to this pipeline.

The following product/data invariant is accepted as authoritative for this review:

> Consecutive key sends that are not members of the same authored chord are not
> pathologically close. Song JSON files already guarantee adequate spacing.

Consequences:

- no new release-gap model;
- no new tempo correction or authored-onset shifting for ordinary notes;
- no new scheduler margin for ordinary consecutive notes;
- no change to the accepted same-key equality/degraded-conflict contract;
- no cross-event collision solver around chord stagger.

This report therefore supersedes the earlier recommendation to change same-key
equality handling. That path is explicitly not actionable here.

P0 remains immutable: `SendInput` only; no game memory, hooks, injection, debugger
attachment, process tampering, or anti-cheat bypass.

## 2. Evidence and limits

Evidence used:

1. current source and tests;
2. deterministic scheduler/estimator reproductions;
3. `tracemalloc` allocation measurements;
4. the Microsoft `SendInput` API contract;
5. canonical timing/architecture documents, with conflicts called out.

The review does **not** have game-observed audio/onset capture. Sender completion,
visible-lateness, and immediate-retry timing must not be presented as proof that the
game sampled an event in a particular frame.

Verification already performed:

```text
119 focused scheduler/dispatch/cache/memory tests passed
2 dispatch-audit baseline tests passed
security-mandate audit passed
tracked worktree was clean before this report was written
```

## 3. Current execution flow

```text
Song JSON
  -> note drafts + strict parsing
  -> hold/release planning
  -> exact-timestamp grouping into KeyAction batches
  -> optional post-build chord DOWN stagger
  -> RuntimeSchedule generation compilation
  -> RuntimeDispatchCoordinator deadline/pending-release state
  -> HybridWaitStrategy timer/event/spin
  -> DispatchLoop due drain
  -> WinSendInputBackend state tracking
  -> one SendInput batch per chord
       -> at most one immediate, sleepless retry after partial note-on
  -> completion-anchored release floor
```

The main architectural choices are sound:

- domain/orchestration remain independent of Win32;
- the dispatch thread is the sole backend sender;
- minimum hold is anchored to down-dispatch completion;
- normal chords use a single `SendInput` call;
- coordinator live state is bounded by polyphony;
- high-resolution event wait avoids continuous polling in the normal path.

## 4. Residual findings

### R1 — High: chord stagger can schedule a chord's UP before some DOWNs

`apply_chord_stagger()` splits and shifts only multi-key DOWN batches
(`src/sky_music/domain/scheduler.py`, `apply_chord_stagger`, lines 123–168).
Releases are intentionally left at their original time. The transform runs after hold
planning and metrics (`build_key_actions`, lines 410–417).

The profile parser permits any non-negative step/cap and does not relate the maximum
offset to the note hold (`scheduler_types.py`, lines 158–159). The feature is reachable
from production CLI flags `--chord-stagger-us` and `--chord-stagger-max-us`.

Deterministic reproduction:

```text
profile FPS:       144
chord size:        7
effective hold:    9,181 us
stagger step/cap:  2,500 / 15,000 us

DOWN offsets:      0, 2,500, 5,000, 7,500, 10,000, 12,500, 15,000
shared UP offset:  9,181
```

The final three DOWNs occur after their shared UP.

The failure continues through runtime:

1. `compile_runtime_intents()` sees the UP before those three DOWNs and assigns
   `generation_id=None` to their UP intents.
2. The later DOWNs receive new generations with no future authored UP.
3. `RuntimeDispatchCoordinator.is_finished()` checks cursor and pending releases, but
   not active generations.
4. Final abort cleanup releases the late keys almost immediately.

This is a chord-local lifecycle violation, not a close-ordinary-notes problem.

Existing tests do not cover the unsafe range:

- scheduler tests explicitly assert that releases are untouched;
- the golden stagger fixture uses a 30 ms hold and only a 6 ms spread, so every DOWN
  remains before the UP.

Required outcome:

- every staggered key retains `up_us >= down_us + effective_min_hold_us`;
- no generated musical DOWN lacks a matching future UP;
- ordinary non-chord schedules remain byte-for-byte unchanged.

### R2 — High but exceptional: join timeout still permits close-under-live-thread

The 2026-07-23 hardening correctly added cooperative shutdown:

1. enqueue quit;
2. signal command event;
3. join with a five-second timeout.

Residual: after the timeout, the supervisor closes the command event even if the
dispatch thread is still alive (`playback_supervisor.py`, lines 500–512).

The outer engine then assumes the join completed and may:

- close the realtime sleeper/waitable timer;
- clear the shared ctypes INPUT-array cache;
- clear runtime references;
- run full GC.

The lifecycle test only covers a cooperative fake that consumes quit. It verifies
`join` occurs before `close`, but does not cover `join` returning while
`is_alive()` remains true.

This path is rare, but its failure mode is more serious than a bounded handle leak:
the dispatch thread can wait on a closed/reused handle or race cache teardown.

Required outcome:

- shared handles/cache are never closed or cleared while the dispatcher is alive;
- timeout becomes an explicit fatal lifecycle result;
- no attempt is made to kill a Python thread or send input from the control thread.

### R3 — Medium-high: lead-cache import is not closed under export

`SendLatencyEstimator.update_completion_error()` legally produces negative residual
EMA values. `export_state()` writes them unchanged, but `import_state()` accepts only
`0 <= ema_residual <= 500`.

Reproduction:

```text
exported ema_residual = -100.0
import_state(exported_state) = False
```

Thus a normal application-generated cache can silently cold-start the next session.
Adaptive lead is enabled by default in production, so this is not a test-only seam.

The inverse validation problem also exists. `count_down` and `sum_down` are checked
only for list type and length. A string element is accepted and copied into estimator
state:

```text
malformed count_down[1] = "x"
import_state(...) = True
next update(...) -> TypeError
```

The poison test covers invalid EMA values and list lengths, but not element types or
the negative-residual round trip.

Required outcome:

- every state produced by `export_state()` imports successfully into a compatible
  estimator;
- malformed array/scalar element types are rejected before mutating estimator state;
- failed import leaves the estimator unchanged.

TTL/age policy is not required for this fix. `saved_at` handling can remain a separate
product decision.

### R4 — Medium: `WAIT_FAILED` can become a full-gap busy spin

The Win32 wrapper maps `WAIT_FAILED` to `None`, which is an acceptable boundary
representation if every caller treats it as degradation/error.

In the high-resolution event branch, `HybridWaitStrategy.wait_until_us()` checks only
for `WAIT_OBJECT_0 + 1`. A `None` result falls through to `spin_until_us(target)`.
For a long inter-note gap, one invalid handle can therefore turn the whole remaining
gap into a 100% CPU spin and prevent timely command polling.

Current tests cover:

- ctypes prototypes;
- wrapper conversion `WAIT_FAILED -> None`;
- non-high-resolution degraded command wait.

They do not cover `None` returned inside the high-resolution two-handle branch.

Required outcome:

- `WAIT_FAILED` never enters full-gap spin;
- degrade to bounded sleep/poll or raise a controlled wait error;
- normal timer/event success path remains unchanged.

### R5 — Medium: core warmup is placed after the normal deadline wait

The normal loop:

1. reads `next_deadline_us`;
2. waits until that deadline;
3. obtains current elapsed time;
4. calls `_drain_due()`.

The warmup block is at the start of `_drain_due()` and queries
`next_deadline_us()` before popping the current due item. In the normal path, the
coordinator cursor still points to the just-due item, so:

```text
remaining_budget = current_deadline - now <= 0
```

The warmup can run after an early/spurious event wake, but it is not reliably executed
before the first send after a normal cold gap. The current positive-budget unit test
uses a mocked future deadline while `_drain_due()` has no due action, which does not
model the normal `run()` sequence.

This is primarily an ineffective optimization, not a send-correctness failure.

Required outcome:

- a real-coordinator/fake-clock test proves whether warmup runs before the first
  post-idle send;
- if retained, warmup must occur inside the wait phase with a budget that leaves the
  final deadline guard intact;
- do not enlarge spin budgets as part of the correction.

### R6 — Medium-low and deferred: one stale `now_us` is reused across a due drain

`_drain_due()` snapshots `now_us`, materializes all due authored batches, then passes
the same value to every `_dispatch_down_batch()`.

Most important timing data are refreshed elsewhere:

- `SendInput` start/completion timestamps;
- telemetry actual time;
- post-UP pending release time;
- focus signal/probe.

The remaining behavioral effect is mainly optional late-pulse-drop. No production
caller currently enables that option. This should not trigger a scheduler rewrite or
new song-spacing model. A single fresh clock read before an enabled late-drop decision
is sufficient if this seam is activated later.

### R7 — Low: partial-chord semantics are more certain in comments than in evidence

Normal behavior is appropriate: send the whole chord in one `SendInput` call.
Microsoft documents that INPUT events in one call are inserted serially and are not
interspersed with other keyboard/mouse input. It does not promise that a game samples
two separate `SendInput` calls in the same frame.

After a partial note-on, the code makes one immediate, sleepless retry. Keeping this
policy is reasonable for accuracy: dropping the remainder guarantees missing notes,
while a prompt retry often recovers them.

Residual issues:

- comments call the retry “SAME-FRAME” and cite an unmeasured 99.7% heuristic;
- two docstrings still say note-on never retries;
- a retry-recovered chord appears fully sent to the coordinator, while split recovery
  is visible only in aggregate diagnostics.

Required outcome:

- no retry-policy behavior change;
- wording must say “immediate retry, likely same-frame under measured sender
  conditions,” not guarantee game-frame observation;
- optional action-level split-recovery telemetry must remain off the common success
  hot path.

### R8 — Low: canonical timing documents disagree

`timing-principles.md` describes the current code:

- default 500 us device-delivery margin;
- `audience_safe` frame multiplier 1.5.

`timing-profile-frame-model.md` still says:

- no fixed margin;
- `audience_safe` multiplier 1.1.

This does not change runtime directly, but it makes future AI review/calibration unsafe
because both files are listed as canonical. The frame-model document must be reconciled
with current code and `timing-principles.md`; no profile-number change is proposed.

## 5. CPU assessment

Normal-path CPU choices are mostly appropriate for an accuracy-first player:

- event-driven wait is enabled in production;
- final guard spin is short and intentional;
- no yield should be added inside the pure final spin;
- batching a chord into one `SendInput` reduces syscalls and timing skew;
- array prewarm removes hot-path ctypes construction for known shapes.

The main abnormal CPU risk found is R4: `WAIT_FAILED` falling into full-gap spin.

The supervisor's roughly 10 ms control cadence and 20–50 ms focus cadence add wakeups,
but dispatch is isolated on its own thread and this is not currently a top optimization
target.

## 6. RAM assessment

Synthetic `tracemalloc` measurements on the current free-threaded interpreter:

```text
50,000 KeyAction objects:
  source actions                         5.72 MiB
  RuntimeSchedule incremental           13.71 MiB
  combined current                      19.42 MiB

8,192 distinct cached ctypes arrays:
  incremental                            7.13 MiB
```

Interpretation:

- runtime compilation is a linear duplicate representation, not an unbounded leak;
- the ctypes cache is bounded and cleared after normal playback;
- AOT compilation and prewarm directly protect deadline accuracy.

Do not remove either mechanism for generic RAM thrift. Before changing representation,
measure real largest-song peak allocation and UI responsiveness. A compact runtime
representation can be a later independent project only if real data justifies it.

## 7. Priority

Recommended order:

1. R1 chord lifecycle correctness.
2. R2 shutdown-timeout ownership.
3. R3 lead-cache validation/round-trip.
4. R4 `WAIT_FAILED` degradation.
5. R5 warmup placement and real-path test.
6. R7/R8 documentation and observability hygiene.
7. RAM representation work only after real-song profiling.

R6 remains deferred unless late-pulse-drop receives a production caller.

## 8. Final assessment

The core does not need a rewrite. The scheduler/coordinator/platform separation,
completion anchor, event wait, bounded live generation state, and one-call chord send
are good foundations.

The highest-impact defect is narrowly defined: the optional post-build chord transform
breaks per-key lifecycle ordering. The remaining important work is at exceptional
resource ownership and cache/wait boundaries. Ordinary non-chord note spacing is not a
problem to solve and must stay outside the implementation scope.
