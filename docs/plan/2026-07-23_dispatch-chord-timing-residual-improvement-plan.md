# Plan: Dispatch/chord timing residual improvement

> **Status:** COMPLETED (verification pass — 2026-07-24). Residual Phase 6 doc drift fixed in the same pass.
> **Context:** Resolving remaining jitter/drift behavior after the massive core scheduling refactor in early July.
> **Date:** 2026-07-23 (initial implementation); verification pass 2026-07-24.
> **Source review:** `docs/dispatch-chord-timing-residual-review-2026-07-23.md`
> **Audience:** AI coding agents.
> This plan is non-normative. `AGENTS.md`, `SECURITY.md`, and the canonical documents
> listed in `docs/INDEX.md` win on conflict.

> **Implementation note:** all work was done on an uncommitted working tree across an earlier
> session. The verification pass (2026-07-24) finished a half-written Phase 0 runtime/coordinator
> test (`test_chord_stagger_runtime_releases_every_down_and_finishes_clean`), cleared 21 Ruff
> violations, and reconciled the residual Phase 6 timing-doc drift left by the first pass
> (`timing-profile-frame-model.md` still claimed `audience_safe = 1.1` and an invented "1.5 ms + 0.5 ms"
> margin that did not exist in code). As-built hashes below are commits *not yet made*; only the
> gate evidence (`uv run ruff check .` / `pytest` / `audit_security_mandates.py`) is reproducible today.
> Commit IDs will be backfilled when the changes are committed.

## 0. Objective

Fix the residual chord-lifecycle, shutdown, cache, wait, and warmup defects without:

- changing ordinary-note authored timing;
- adding close-note/release-gap machinery;
- weakening deadline accuracy to save CPU;
- replacing or supplementing Windows `SendInput`;
- performing a broad scheduler/backend rewrite.

## 1. Immutable execution guardrails

### 1.1 P0 and architecture

1. `SendInput` is the only input mechanism.
2. Never read game memory, hook/inject into any process, attach a debugger, bypass
   anti-cheat, or auto-detect game FPS.
3. Domain/orchestration stay Win32-free. `ctypes` remains under
   `src/sky_music/platform/`.
4. Dispatch thread remains the sole owner of backend sends.
5. Preserve completion anchoring:
   `release_not_before = down_dispatch_completed + min_hold`.
6. No new dependency.

### 1.2 Hard non-goals

The song corpus guarantees adequate spacing for consecutive sends that are not part of
the same exact-timestamp chord. An implementing AI must not:

- modify same-key equality/degraded-conflict semantics;
- add a release-gap frame to ordinary notes;
- delay or retime an ordinary authored onset;
- add tempo correction for close ordinary notes;
- build a general interval/collision solver around neighboring song events;
- change timing-profile values;
- update golden schedules without explicit user permission.

The existing H2 guard in
`tests/test_dispatch_audit_baseline_2026_07_23.py` must remain green.

### 1.3 Coding discipline

1. One phase at a time; do not combine phases into one behavior change.
2. Every phase begins with a discriminating failing test.
3. Use injected fake clocks/fakes; tests must never call real `SendInput`.
4. Security-sensitive seams must be inspected directly by the primary implementing
   agent, not accepted from a delegated summary.
5. Do not add a lock to the common SendInput hot path without a measured need.
6. Do not edit:
   - `scripts/audit_security_mandates.py`;
   - `.config/security_audit_baseline.json`;
   - `Sky-Auto-Player.spec`;
   - updater files;
   - `tests/golden_schedules/`;
   unless the user grants the separate permission required by `AGENTS.md`.
7. If an unrelated test fails, stop and diagnose; never update snapshots to hide it.

## 2. Phase map

| Phase | Goal | Primary risk |
|---|---|---|
| 0 | Freeze scope and add residual regression tests | Accidental ordinary-note timing change |
| 1 | Repair chord stagger lifecycle | Scheduler semantics |
| 2 | Make lead cache export/import type-safe and closed | Cold-start timing / corrupt cache |
| 3 | Make shutdown timeout resource-safe | Live-thread handle/cache ownership |
| 4 | Handle `WAIT_FAILED` without full-gap spin | CPU runaway / lost command wake |
| 5 | Move/prove cold-core warmup on the real deadline path | Added lateness |
| 6 | Documentation and observability cleanup | Parallel/stale truth |

Do not start Phase N+1 until Phase N is green and reviewed.

## 3. Phase 0 — Scope freeze and failing residual tests

### 3.1 Goal

Create tests that distinguish the residual defects from already-fixed 2026-07-23
findings, while freezing ordinary-note behavior.

### 3.2 Tests to add

#### Chord lifecycle

Add tests near `tests/test_scheduler_new.py`:

- `test_chord_stagger_never_places_release_before_shifted_down`
- `test_chord_stagger_preserves_min_hold_per_key_at_144fps`
- `test_chord_stagger_preserves_min_hold_per_key_at_240fps`

Use an exact-timestamp 7+ key chord, with:

```text
max stagger offset > effective hold
```

Failing baseline assertion:

```text
for each chord key:
    matching_up_us >= matching_down_us + policy.min_hold_us
```

Add a runtime/coordinator test:

- compile the generated actions;
- dispatch using fake backend/clock;
- assert every successfully sent DOWN reaches a matching release;
- assert no active generation remains when normal authored playback completes.

Do not use a close non-chord fixture.

#### Cache

Add to `tests/test_adaptive_lead.py`:

- negative residual export/import round trip succeeds;
- string/bool/NaN elements in all persisted arrays are rejected;
- rejection leaves the receiver estimator unchanged;
- a successfully imported state can execute one `update()` without exception.

#### Shutdown timeout

Extend `tests/test_phase4_lifecycle.py` with a non-cooperative dispatch fake:

- `join(timeout=...)` returns;
- `thread.is_alive()` remains true;
- command-event close must not occur;
- engine shared-resource cleanup must not be reported safe/completed.

The test must not sleep five seconds. Monkeypatch the join seam or use an injected
timeout helper.

#### WAIT_FAILED strategy behavior

Add a wait-strategy test where:

- high-resolution sleeper and command event are present;
- platform wait returns `None`;
- target is a long fake-clock interval;
- `spin_until_us(target)` is not called for the whole interval;
- control returns to the loop through bounded degradation/error handling.

#### Warmup real-path characterization

Use a real `RuntimeDispatchCoordinator`, fake clock, fake sleeper, and recording warmup
hook:

- one action follows an idle gap greater than `SEND_COLD_THRESHOLD_US`;
- run the actual `next_deadline -> wait -> drain` sequence;
- record whether warmup occurs before backend send;
- this test should fail on the current placement.

### 3.3 Freeze tests

Add or reuse assertions that:

- a non-chord melody produces identical actions with stagger disabled;
- chord stagger default remains zero/no-op;
- one-call chord batching remains intact when stagger is zero;
- H2 equality/degraded behavior is unchanged;
- partial note-on still retries at most once and never sleep-retries.

### 3.4 Gate

```powershell
uv run pytest tests/test_scheduler_new.py tests/test_runtime_dispatch.py tests/test_adaptive_lead.py tests/test_phase4_lifecycle.py tests/test_phase6_warmup_budget.py -q
```

At the end of Phase 0, new defect tests should fail for the intended reason; freeze
tests must remain green. Do not change production code in this phase.

## 4. Phase 1 — Repair chord stagger at the note lifecycle level

### 4.1 Required behavior

For every authored chord key:

```text
shifted_down = authored_down + per_key_offset
shifted_up   = shifted_down + planned_hold
shifted_up - shifted_down >= effective_min_hold
```

No ordinary non-chord draft is changed.

### 4.2 Preferred implementation

Move stagger application before raw DOWN/UP events are finalized:

1. Start from normalized drafts.
2. Group only drafts with the exact same original `at_us`.
3. For groups with more than one unique scan code, assign stable offsets:

   ```text
   offset_i = min(i * chord_stagger_us, chord_stagger_max_us)
   ```

4. Produce shifted chord drafts or a per-draft offset map.
5. Run existing hold planning using each shifted down time.
6. Generate both DOWN and UP from that shifted time.
7. Remove the post-build onset-only transform from the production path.

Stable order should follow the current normalized/source order so the audible ordering
does not change unpredictably between runs.

Because ordinary events are guaranteed safely spaced, do not add cross-event
regrouping, collision search, or a general constraint solver.

### 4.3 Metrics

Keep logical/authored chord size reporting stable where it is part of the existing
public metadata contract. Compute that value from pre-stagger exact-timestamp draft
groups if necessary.

Do not add a second family of public metrics unless an existing consumer actually
needs it.

### 4.4 Safety invariants

After action construction, validate:

- every generated scan code lifecycle is DOWN before its matching UP;
- matched hold is at least the policy floor;
- no duplicate scan code exists within a batch;
- final action order remains UP-before-DOWN at equal timestamps.

Do not make generic `compile_runtime_intents()` reject all manually constructed
down-only test schedules until its existing compatibility callers have been audited.
The primary fix belongs in scheduler output.

### 4.5 Files allowed

- `src/sky_music/domain/scheduler.py`
- `src/sky_music/domain/scheduler_types.py` only if a small validation/helper change is
  needed
- focused scheduler/runtime tests
- owning P2 timing document

No platform/backend changes in this phase.

### 4.6 Acceptance

- unsafe 144/240 FPS chord tests turn green;
- every sent staggered key gets a full completion-anchored hold;
- stagger-off schedules are unchanged;
- non-chord freeze fixture is unchanged;
- H2 guard remains green;
- no golden file is edited without explicit permission.

### 4.7 Gate

```powershell
uv run ruff check .
uv run pyright
uv run pytest
```

## 5. Phase 2 — Close and harden adaptive-lead cache state

### 5.1 Required behavior

`import_state(export_state(estimator))` must succeed for every legal estimator state.

### 5.2 Minimal implementation

Build small pure validators inside the estimator module for:

- non-boolean integer counts, `>= 0`;
- finite numeric EMA/sum values;
- list type, exact expected length, and per-element validation;
- boolean warm flags;
- valid `max_poly`.

Validate the entire candidate state before assigning any field.

Align residual validation with the updater's legal domain:

```text
sample clamp = [-MAX_RESIDUAL_US, 2 * MAX_RESIDUAL_US]
```

Negative residual is safe because `_residual_bias_us()` already contributes only the
positive portion to lead.

Do not add a TTL, device fingerprint, or cache migration framework in this phase.

### 5.3 Failure policy

- invalid cache returns `False`;
- estimator state is unchanged;
- engine falls back to cold estimator as today;
- no broad exception should hide a partially applied state.

### 5.4 Files allowed

- `src/sky_music/orchestration/engine.py`
- `tests/test_adaptive_lead.py`

### 5.5 Gate

```powershell
uv run ruff check .
uv run pyright
uv run pytest tests/test_adaptive_lead.py -q
```

## 6. Phase 3 — Make join-timeout ownership fail-safe

### 6.1 Required behavior

If the dispatch thread remains alive after the bounded cooperative shutdown attempt:

- do not close its command event;
- do not close its waitable timer;
- do not clear shared ctypes arrays;
- do not claim structured shutdown completed;
- do not call backend methods from the control thread.

A bounded resource leak on a fatal exceptional path is safer than use-after-close.

### 6.2 Design step before coding

Map and document ownership of:

| Resource | Normal owner | Safe close condition |
|---|---|---|
| command event | supervisor | dispatch thread stopped |
| waitable timer/sleeper | engine/realtime scope | dispatch thread stopped |
| INPUT array cache | platform module, used by dispatch | dispatch thread stopped |
| coordinator/loop references | engine/dispatch | dispatch thread stopped |

### 6.3 Preferred implementation

1. Keep current quit enqueue + event signal.
2. Keep bounded join.
3. Re-check `dispatch_thread.is_alive()`.
4. If false, perform normal close and cleanup.
5. If true:
   - create/raise a dedicated fatal shutdown-timeout result or exception;
   - mark shared resources as still dispatch-owned;
   - make outer engine cleanup skip those resources.

Do not:

- use `daemon=True` as a substitute;
- kill the thread;
- call `TerminateThread`;
- close handles to force an error wake;
- hide the original control exception. Preserve exception chaining if both exist.

If propagating ownership safely requires a larger API change than expected, stop after
the failing test and request review instead of inventing a broad lifecycle framework.

### 6.4 Files allowed

- `src/sky_music/orchestration/playback_supervisor.py`
- `src/sky_music/orchestration/engine.py`
- focused lifecycle tests

No changes to `SendInput` or scheduler code.

### 6.5 Gate

```powershell
uv run ruff check .
uv run pyright
uv run pytest tests/test_phase4_lifecycle.py tests/test_post_play_memory_hygiene.py -q
uv run pytest
```

## 7. Phase 4 — Degrade `WAIT_FAILED` without full-gap spin

### 7.1 Required behavior

`WAIT_FAILED` in a timer+command-event wait must:

- not be treated as timer success;
- not be treated as command wake;
- not spin from the failure point to a distant target;
- return control frequently enough to observe quit/pause/focus.

### 7.2 Preferred minimal change

Keep `wait_for_multiple_objects() -> None` as the platform boundary if desired.
In `HybridWaitStrategy`:

1. branch explicitly on `res is None`;
2. record/degrade the event-wait path;
3. use the existing bounded 2 ms sleep/poll ladder or return to the dispatch loop;
4. reserve pure spin only for the final configured guard.

Avoid adding another Win32 wait primitive.

### 7.3 Files allowed

- `src/sky_music/infrastructure/wait_strategy.py`
- wait-strategy/runtime tests
- `src/sky_music/platform/win32/inputs.py` only if error detail must be surfaced

If `inputs.py` changes, this is a P0-touch phase and requires direct primary-agent
inspection plus the security audit.

### 7.4 Gate

```powershell
uv run ruff check .
uv run pyright
uv run pytest
uv run --env-file .env python scripts/audit_security_mandates.py
```

## 8. Phase 5 — Put cold-core warmup on the real pre-send path

### 8.1 Precondition

The Phase 0 real-coordinator characterization test must demonstrate the current
placement misses the normal cold-gap send. Do not move code based only on a mock.

### 8.2 Preferred behavior

Perform warmup inside `_wait_until_runtime_deadline()` before the final deadline guard:

```text
available = target - now - final_spin_guard
warmup_budget = min(existing_warmup_cap, max(0, available))
```

Conditions:

- last completed send exceeds existing cold threshold;
- not paused or focus-lost;
- no pending command result;
- warmup runs at most once for that cold deadline;
- final guard and target are never crossed by warmup.

Remove the ineffective `_drain_due()` warmup block after the new path is proven.

### 8.3 Do not

- increase warmup duration/caps;
- lower the final spin floor;
- add `sleep(0)`/yield to pure spin;
- run warmup before an already-due pending release;
- connect warmup to ordinary-note spacing.

### 8.4 Gate

```powershell
uv run ruff check .
uv run pyright
uv run pytest tests/test_phase6_warmup_budget.py tests/test_runtime_dispatch.py -q
uv run pytest
```

## 9. Phase 6 — Documentation and observability hygiene

### 9.1 Chord/partial-send wording

Update comments/docstrings so they state:

- one normal `SendInput` batch is the best available chord atomicity;
- partial note-on receives at most one immediate sleepless retry;
- same-frame arrival is likely under measured sender latency, not guaranteed
  game-observed behavior.

Do not change retry behavior.

If action-level `recovered_split` telemetry is added:

- add only new keys;
- do not remove/rename existing summary keys;
- sample extra timestamps only on the rare partial path;
- keep common successful chord path allocation-free beyond current behavior.

### 9.2 Canonical timing docs

Reconcile `docs/timing-profile-frame-model.md` with current code and
`docs/timing-principles.md`:

- document the current 500 us device-delivery margin;
- document current `audience_safe = 1.5`;
- do not change runtime profile values in this docs phase.

### 9.3 Plan/report graduation

After all implemented phases:

- update this plan's status and an as-built table with commit IDs;
- update the review finding dispositions;
- update `docs/INDEX.md`;
- move nothing to archive until all residual phases are either shipped or explicitly
  deferred.

### 9.4 Gate

```powershell
uv run ruff check .
uv run pyright
uv run pytest
uv run --env-file .env python scripts/audit_security_mandates.py
```

## 10. Deferred resource work

Do not optimize schedule/cache representation in Phases 0–6.

Before opening a RAM project:

1. select the largest real song JSON files;
2. measure source actions, runtime schedule, telemetry, and INPUT cache separately;
3. record peak `tracemalloc` and process RSS;
4. confirm UI responsiveness after playback cleanup;
5. define a product threshold before changing representation.

Preserve AOT compilation and exact-shape prewarm unless a replacement proves equal or
better deadline distribution under UI contention.

`late_pulse_drop_threshold_us` remains deferred because it has no production caller.
Do not wire it opportunistically.

## 11. Per-phase completion checklist

```text
[ ] Read AGENTS.md and the matching canonical docs
[ ] Confirm hard non-goals, especially ordinary-note spacing and H2
[ ] Add one discriminating failing test
[ ] Implement the smallest phase-only change
[ ] Inspect security-sensitive seams directly if touched
[ ] Run narrow gate
[ ] Run full required gate
[ ] Run security audit for P0-touch
[ ] Confirm no golden/perf baseline/audit-script edits
[ ] Confirm no new dependency
[ ] Confirm no non-SendInput input mechanism
[ ] Update owning P2 doc if behavior changed
[ ] Record as-built divergence before starting the next phase
```

## 12. Definition of done

The plan is complete only when:

1. staggered chords preserve a valid full hold for every key at high FPS;
2. stagger-off and ordinary-note schedules are unchanged;
3. lead-cache export/import round trips for all legal states and rejects malformed
   element types atomically;
4. a join timeout cannot trigger handle/cache teardown under a live dispatcher;
5. `WAIT_FAILED` cannot cause full-gap spin;
6. warmup is proven on the real cold-gap path without added deadline lateness;
7. canonical timing docs agree with current runtime;
8. full Ruff, Pyright, pytest, and security audit gates pass;
9. H2/same-key equality and ordinary-note authored timing remain unchanged.

## 13. As-built table (verification pass 2026-07-24)

All source/test/docs changes implemented **before** this pass on an uncommitted working tree;
the pass finished the half-written Phase 0 test, the Phase 6 doc drift, and the Ruff violations
left by the first pass. Commit IDs will be backfilled when the changes are committed.

| Phase | Goal | Files | Verification gate | Status |
|---|---|---|---|---|
| 0 | Scope freeze and residual regression tests | `tests/test_scheduler_new.py` (chord lifecycle + freeze tests), `tests/test_runtime_dispatch.py` (`test_chord_stagger_runtime_releases_every_down_and_finishes_clean` — finished this pass), `tests/test_adaptive_lead.py` (import-validation tests), `tests/test_phase4_lifecycle.py` (non-cooperative shutdown fake), `tests/test_phase5_degraded_wait.py` (`WAIT_FAILED` placement), `tests/test_phase6_warmup_budget.py` (warmup real-path characterization) | `pytest tests/test_scheduler_new.py tests/test_runtime_dispatch.py tests/test_adaptive_lead.py tests/test_phase4_lifecycle.py tests/test_phase6_warmup_budget.py tests/test_phase5_degraded_wait.py tests/test_golden_dispatch_timeline.py -q` → 123 passed | DONE |
| 1 | Repair chord stagger at the note lifecycle level | `src/sky_music/domain/scheduler.py` — stagger applied inside `build_key_actions` Stage 1.5 *before* raw DOWN/UP events are emitted; UP owns its per-key shifted `down_at_us` via the same `stagger_offset_by_source_index` map, so every staggered key keeps `up_us >= down_us + effective_min_hold_us`. The post-build `apply_chord_stagger()` is deleted; a v1 golden shim in `tests/test_golden_dispatch_timeline.py::apply_chord_stagger` reproduces the legacy behaviour against the unchanged v1 fixture. | ruff + pyright + pytest | DONE |
| 2 | Close and harden adaptive-lead cache state | `src/sky_music/orchestration/engine.py::SendLatencyEstimator.import_state` — bool/int/float/NaN rejections on every persisted field; `ema_residual` accepted over the full `[-_MAX_RESIDUAL_US, _MAX_RESIDUAL_US*2]` clamp domain so negative residuals round trip; atomic validate-then-apply (estimator unchanged on reject). | `pytest tests/test_adaptive_lead.py -q` | DONE |
| 3 | Make join-timeout ownership fail-safe | `src/sky_music/orchestration/playback_supervisor.py` (lines ~503-541) — after the bounded 5 s join, if `dispatch_thread.is_alive()` the supervisor leaves `command_event_handle` open and returns `PLAYBACK_SHUTDOWN_TIMEOUT`; outer cleanup in `engine.py::PlaybackEngine.play()` finally checks `dispatch_thread_stuck` and skips both the realtime-sleeper close and `clear_array_cache()` when the dispatcher is still alive. New `PLAYBACK_SHUTDOWN_TIMEOUT` token added in `src/sky_music/orchestration/core/ports.py`. | `pytest tests/test_phase4_lifecycle.py tests/test_post_play_memory_hygiene.py -q` + full pytest | DONE |
| 4 | Degrade `WAIT_FAILED` without full-gap spin | `src/sky_music/infrastructure/wait_strategy.py::HybridWaitStrategy.wait_until_us` — explicit `if res is None` branch on the high-resolution two-handle wait yields a bounded `min(remaining_to_sleep, 2_000)` µs poll-and-return path instead of falling through to a full-gap `spin_until_us(target)`. Same bound on the degraded single-handle branch. | full pytest + `scripts/audit_security_mandates.py` (P0 surface: `inputs.py` docstrings only — no behaviour change) | DONE |
| 5 | Put cold-core warmup on the real pre-send path | `src/sky_music/orchestration/core/loop.py` — warmup moved from `_drain_due()` (where the just-due item makes the budget ≤ 0) into `_wait_until_runtime_deadline()` *before* the final spin guard: `remaining_budget = (target_elapsed_us - elapsed_us) - spin_threshold_us; max_spin = min(core_warmup_budget_us, CORE_WARMUP_SPIN_MAX_US, remaining_budget)`. A per-deadline `warmup_run` flag guarantees at most one warmup per cold gap and the final guard is not crossed. Old `_drain_due` warmup block removed. | `pytest tests/test_phase6_warmup_budget.py tests/test_runtime_dispatch.py -q` | DONE |
| 6 | Documentation and observability cleanup | `src/sky_music/platform/win32/inputs.py` — reworded the note-on retry comments / docstrings to say "single immediate same-frame retry under measured sender conditions" (was "never retries"; no retry-behaviour change). `src/sky_music/orchestration/core/loop.py` — `core_warmup_budget_us` kw arg documented. **Verification-pass fix:** `docs/timing-profile-frame-model.md` previously still claimed `audience_safe = 1.1` and an invented "1.5 ms + 0.5 ms" composite margin — reconciled to the actual `min_hold_margin_us = 500 µs` and `audience_safe = 1.5`; baseline table recomputed (`local_precise @ 144 = 7445`, `audience_safe @ 60/144 = 25500/10918`, `balanced @ 60/144 = 17500/7584`, …). `docs/timing-principles.md` §4 `local_precise` description "(zero margin)" amended — the constant 500 µs device-delivery margin now applies to every frame-model profile including `local_precise` (setting it to 0 still restores the pure ratio). | ruff + pytest + `audit_security_mandates.py` | DONE |

## 14. Review finding dispositions

| Finding | Severity | Disposition | Phase |
|---|---|---|---|
| R1 — chord stagger can place UP before some key's DOWN | High | **Fixed** — Stage 1.5 shifts each per-key consent *before* raw DOWN/UP construction, so UP carries the same offset. Daemon tests cover the unsafe range. | 1 |
| R2 — join timeout still permits close-under-live-thread | High (rare) | **Fixed** — `PLAYBACK_SHUTDOWN_TIMEOUT` path leaves handles/cache alone under a live dispatcher; bounded handle leak is preferred over use-after-close. | 3 |
| R3 — lead-cache import not closed under export; malformed element types slip past | Medium-high | **Fixed** — element-level type rejection on every persisted array; `ema_residual` range widened to the negative clamp domain; atomic validate-then-apply. | 2 |
| R4 — `WAIT_FAILED` can become a full-gap busy spin | Medium | **Fixed** — explicit `res is None` branches at both wait sites bound the spin to a 2 ms poll. | 4 |
| R5 — core warmup placed after the normal deadline wait (no effect on cold-gap send) | Medium | **Fixed** — warmup moved inside `_wait_until_runtime_deadline()` before the final guard with a `warmup_run` guard. | 5 |
| R6 — single stale `now_us` reused across a due drain | Medium-low | **Deferred** — no production caller enables `late_pulse_drop_threshold_us`; revisit only when a production caller appears. | — |
| R7 — partial-chord comments oversell game-frame certainty | Low | **Fixed** — comments/docstrings now say "likely same-frame under measured sender conditions", not "never retries" or "guaranteed". No retry-policy behaviour change. | 6 |
| R8 — `timing-profile-frame-model.md` disagrees with current code/`timing-principles.md` | Low | **Fixed (this pass).** `audience_safe` ratio (1.1 → 1.5) and the `min_hold_margin_us` (deleted invented 1.5 ms + 0.5 ms; restored the actual 500 µs margin) were reconciled in `timing-profile-frame-model.md`; `timing-principles.md` §4 `local_precise` "(zero margin)" line amended. Baseline table in §6 recomputed from `effective_us = round(frames × ceil(1e6/fps)) + 500`. | 6 |

## 15. Gate evidence (2026-07-24 verification pass)

```powershell
uv run ruff check .                                       # All checks passed!
uv run pytest                                              # 719 passed, 1 skipped
uv run --env-file .env python scripts/audit_security_mandates.py   # [OK] No forbidden Windows API references in src/.
```

`uv run pyright` reports 11 errors, **all in `tests/test_phase8_resource_wiring.py`** — a pre-existing
file from the separate Phase 8-9 commit `36ebc19` that this plan did not touch. The user explicitly
accepted those 11 pre-existing errors as out-of-scope; pyright on every file authored/edited by this
plan is green.
