> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: nhắc tới release_latency_margin_us đã gỡ khỏi code.

# Playback Flow Hardening & Cleanup Plan

Date: 2026-06-05

Status: Phases A-F implemented on 2026-06-05. Phase G in-game validation **PASSED 2026-06-06**
(start-anchor fix loses no same-key notes in-game @144fps; residual jitter is game-side — see
`timing-experiments.md` §0.4 "PHASE G RESULT"). All phases complete.
Supersedes nothing; complements the completed runtime-hold anchor correction.

> **Superseded by completion-anchor refactor (2026-06-06).** The Phase G start-anchor conclusion is
> historical. Current runtime visibility is completion-to-completion, and scheduler feasibility is
> guarded by `release_latency_margin_us`; see `completion-anchor-refactor-plan.md`.

Related:

- `runtime-hold-refactor-plan.md` (Phases 0–6 done; Phase 7 still open)
- `timing-principles.md` (§7 visibility contract — now `down_dispatch_completed + min_hold`)
- `scheduler-core-architecture-plan.md`
- `timing-experiments.md`

## 0. Decision

The playback core is sound after the anchor correction. Do **not** rewrite it. This plan is a
staged hardening + cleanup pass that removes dead state, de-duplicates logic, makes
`PlaybackEngine.play()` maintainable, and closes the two still-open runtime questions (late-burst
policy and in-game validation).

Each phase must land with the **full suite green** (`uv run pytest`) and must not change the authored
scheduler timeline or any built-in profile value. Phases are independent and individually
revertible; do them in the listed order but each can ship on its own.

## 1. Frozen behavior (all phases)

1. `build_key_actions()` stays pure; golden scheduler snapshots unchanged.
2. `KeyAction.at_us` remains the authored absolute schedule.
3. Visibility floor stays `up_dispatch_started >= down_dispatch_started + min_hold` (the corrected
   anchor); no profile/frame-rounding change.
4. Exact-simultaneous chords stay batched into one SendInput.
5. Windows SendInput-only backend boundary; no game-file/memory access.
6. Public CLI flags and defaults unchanged.
7. Telemetry changes are **additive** unless a field is proven dead.

---

## Phase A — Dead-code removal & generation-status telemetry

Goal: delete write-only state and unreachable helpers; if a piece of dead state is actually useful,
wire it to telemetry instead of deleting it.

### Findings (verified by repo-wide grep)

- `RuntimeDispatchCoordinator.status_by_generation` (+ `GenerationStatus` Literal + `RuntimeSchedule.
  generation_count`): written in 7 places, **never read** — not by any decision, telemetry, or test.
- `PreciseSleeper.sleep_until_us`: defined, referenced **nowhere** (not even tests/scripts).

### Changes

1. **Repurpose, don't just delete, `status_by_generation`.** It is the natural source for the
   summary counters `runtime-hold-refactor-plan.md` §9.3 listed but never wired:
   - Add `RuntimeDispatchCoordinator.generation_status_counts() -> dict[str, int]` returning counts
     per `GenerationStatus`.
   - In `PlaybackEngine.play()` finalizer, call it and pass to a new
     `TelemetryLogger.record_generation_status_counts(...)`.
   - Surface in `get_summary()` as `cancelled_generation_count`, `dropped_conflict_count`,
     `dropped_backend_count`, `released_count` (additive keys).
   - If, after review, the counts duplicate existing summary fields exactly, delete
     `status_by_generation` + `GenerationStatus` + `generation_count` outright instead.
2. **Delete `PreciseSleeper.sleep_until_us`** and its docstring. Confirm `spin_until_us` and
   `sleep_step_towards_us` remain the only public stepping API.

### Tests

- New: `generation_status_counts` reflects a known mix (1 released, 1 dropped_conflict, 1 cancelled
  via pause) in a deterministic fake-clock run.
- New: summary exposes the additive counters and they sum consistently with
  `runtime_conflict_dropped_down_count`.
- Existing suite green; no scheduler/golden change.

### Gate / rollback

- No timing behavior change. Revert = restore the deleted helper / drop the new counters.

---

## Phase B — Telemetry de-duplication

Goal: remove duplicated derivations introduced incrementally.

### Changes

1. **Collapse the redundant hold metric.** After the anchor correction,
   `confirmed_hold_lower_bound_us` is now identical to `note_hold_duration_us` (both start-to-start).
   Pick one:
   - Preferred: keep `note_hold_duration_us` as the observed hold and redefine
     `confirmed_hold_lower_bound_us` as the **worst-case** bound `up_dispatch_completed -
     down_dispatch_started` (still ≥ min_hold by construction, but distinct and diagnostic), OR
   - Drop `confirmed_hold_lower_bound_us` and keep only `confirmed_hold_shortfall_count` computed
     against `note_hold_duration_us`.
   - Document the chosen meaning in `timing-principles.md` §7.
2. **Remove the hand-rolled lateness counters in `engine.play()`** (`late_events_over_2/5/10ms`,
   `max_lateness_us`, the `observe_result` closure's counting). `telemetry.get_summary()` already
   computes `over_2ms/5ms/10ms` and lateness percentiles. The end-of-playback `debug_log` line
   should read those from the summary instead. Keep the renderer `update_counters` call (HUD needs
   live values), but feed it from the dispatch result directly without the parallel accumulators.

### Tests

- Summary lateness counters unchanged for an existing fixture (the numbers must match what the old
  hand-rolled counters produced).
- `confirmed_hold_*` semantics test updated to the chosen definition.

### Gate / rollback

- No timing change. Telemetry numbers identical except the intentionally-redefined hold field.

---

## Phase C — `PlaybackEngine.play()` maintainability refactor (pure structural)

Goal: shrink the 120-line method and kill the awkward 3-tuple threading, with **zero** behavior
change (equivalence-tested).

### Changes

1. Introduce a private `@dataclass` `LoopState` holding `last_runtime_poll_us`,
   `last_render_time_us`, `first_action_executed`. `_wait_until_runtime_deadline` mutates it instead
   of returning `(cmd, last_runtime_poll_us, last_render_time_us)`; it returns only
   `command_result: str | None`.
2. Extract `_drain_due(now_us, state) -> Iterable[ExecutionResult | None]` containing the
   `pop_due_pending` + `pop_due_authored` + up/down branch block (`engine.py:566-580`). `play()`
   loops: wait → drain → observe.
3. Move the end-of-playback diagnostic summary out of `play()` into a small
   `_log_timing_summary()` helper (reads from telemetry per Phase B).

### Tests

- This is the highest-risk-for-regression phase despite being "just structural". Gate it with an
  **equivalence test**: the existing deterministic fake-clock scenarios must produce byte-identical
  `TimedBackend.calls` and identical summaries before/after. Run the full
  `tests/test_runtime_dispatch.py`, `test_playback.py`, `test_engine_refactor.py` plus the audit
  bench (`scripts/audit_pipeline_bench.py`) and diff key metrics (drift, drops, shortfall, sent
  counts).

### Gate / rollback

- Deterministic dispatch equivalence required. Revert is a single-commit rollback (no API change
  outside the engine module).

---

## Phase D — Backend logic de-duplication

Goal: remove the near-identical `key_down`/`key_up` state-tracking duplicated between
`WinSendInputBackend` and `DryRunBackend` so the two can't drift.

### Changes

- Extract the active/possibly-active/failed-release set bookkeeping + dedup decisions into a base
  class or mixin (`_TrackedKeyState`) that exposes `_decide_down(scan_codes)` /
  `_decide_up(scan_codes)` returning `(to_send, skipped)` and applies the post-send state update via
  a `_emit(to_send, key_up)` hook.
- `WinSendInputBackend._emit` calls `inputs.send_scan_code_batch`; `DryRunBackend._emit` appends to
  `history`. `release_all()` stays per-class (Win32 has the 3-pass + verify logic; DryRun is
  trivial).

### Tests

- Existing backend tests must pass unchanged (`InputSendResult` contract identical: dedup, idempotent
  up, partial chord skip).
- New: a shared parametrized test runs the same down/up/duplicate sequence through both backends and
  asserts identical `InputSendResult` shapes.

### Gate / rollback

- No behavior change; `InputSendResult` outputs identical. Revert = inline the methods again.

---

## Phase E — Focus-loss-during-burst hardening (decision + optional)

Goal: decide whether to stop dispatching the remainder of a due-batch burst once focus is lost
mid-loop (`engine.py:571`).

### Analysis

Currently a multi-batch burst dispatches fully before the next wait re-checks focus, so a few notes
can be sent in the microseconds after focus is lost. This causes **no stuck key** (next wait does
`release_all`) and the game simply ignores unfocused input — so it is cosmetic, at the cost of one
extra `is_active()` Win32 call per batch if "fixed".

### Decision

Default: **do not add a per-batch focus check** (keeps the hot path clean; the documented design
checks focus once per poll interval and bypasses it entirely in the final spin). Record this as an
explicit, intentional non-fix in `timing-principles.md`/code comment so it isn't "rediscovered" as a
bug later. Only revisit if in-game testing shows audible artifacts at focus-loss boundaries.

### Tests

- None (decision only). Add a comment at `engine.py:571` referencing this section.

---

## Phase F — Late-burst recovery policy (the open Phase 6)

Goal: define behavior after a long thread stall when many authored pulses are already expired, so the
engine does not machine-gun expired downs back-to-back. **Hard constraint (already decided):** never
rebase the absolute music clock — only explicit pause/focus-loss may add to pause time.

### Proposed policy

1. Add a config `late_pulse_drop_threshold_us` (default ≈ 1 frame at the active FPS, e.g.
   `frame_us`). When a due **down** batch is more than the threshold late at dispatch time, classify
   it `dropped_expired` instead of sending it:
   - It cannot register in-game on its intended beat anyway (it would collapse into the next frame
     with the following note — the `catch_up_bursts` telemetry already measures this).
   - Its matching authored up becomes `suppressed_stale_up` via the existing generation machinery.
2. **Never** drop a release for being late — releases must always flush (safety > rhythm).
3. Surface `expired_dropped_down_count` and reuse the existing `catch_up_bursts` summary so the
   policy's effect is observable.
4. Keep it OFF by default behind the threshold until in-game validation (Phase G) confirms dropping
   sounds better than collapsing. Document in `timing-principles.md` (new §) and
   `runtime-hold-refactor-plan.md` Phase 6.

### Tests

- Deterministic stall fixture (reuse `OneShotStallingSleeper`/`ScheduledStallingSleeper`): after a
  long stall, downs later than the threshold are `dropped_expired`, not sent; releases still flush;
  `down_timeline_drift_us == 0` (no rebase); unrelated on-time downs after recovery dispatch
  normally.
- Boundary: a down exactly at the threshold is NOT dropped.

### Gate / rollback

- Off-by-default ⇒ zero change to current playback until explicitly enabled. Revert = remove the
  threshold branch.

---

## Phase G — In-game validation of the anchor correction (the open Phase 7)

Goal: confirm on real hardware that the start-anchor fix restores same-key repeats without
under-holding, at the profile/FPS where the bug appeared.

### Procedure (per `timing-experiments.md` §0 method — ground truth = recorded game audio)

1. Lock game FPS externally to 144. Run `local_precise` (`min_hold ≈ 6945 µs`).
2. Songs:
   - A synthetic fast same-key staircase near the floor (`tests/make_test_song.py`) with authored
     intervals at `min_hold`, `min_hold + 0.5 ms`, `min_hold + 1 ms`.
   - One dense real song with rapid same-key passages.
3. For each: record WAV, count onsets (`tests/analyze_onsets.py`), and capture `--debug-csv`.
4. Compare against the **pre-fix** build (completion anchor) on the same songs.

### Acceptance

- Onset count matches authored same-key note count (no intermittent drops) where authored
  `interval ≥ min_hold`.
- `runtime_conflict_dropped_down_count == 0` for these feasible songs.
- No new under-hold artifact (notes still register, i.e. observed hold ≥ ~1 frame).
- p95 send lateness unchanged from the EXP-1 baseline (~0.13 ms).

### Decision gate

Only after Phase G passes should any further profile-margin change be considered (per
`timing-principles.md` §20.8 / Definition-of-Done). If repeats at exactly `min_hold` still drop
occasionally in-game, that is the real-hardware down-dispatch jitter eating the zero headroom — the
fix is correct, the remedy is tempo/profile, not re-introducing the over-hold.

---

## Cross-phase: risks & mitigations

| Risk | Mitigation |
| --- | --- |
| Phase C structural refactor silently changes dispatch order | Deterministic byte-identical `TimedBackend.calls` equivalence gate + audit bench diff |
| Deleting `status_by_generation` removes future-useful observability | Phase A wires it to telemetry first; only delete if proven redundant |
| Telemetry field redefinition (Phase B) breaks calibration | Calibration reads `note_hold_duration_us` (unchanged); confirmed-hold is advisory per refactor-plan §9.4 |
| Late-burst drop (Phase F) removes wanted notes | Off by default; threshold ≈ 1 frame; releases never dropped; validated in Phase G before enabling |
| Backend mixin (Phase D) changes `InputSendResult` semantics | Shared parametrized contract test across both backends |

## Definition of done

1. No write-only runtime state remains (`status_by_generation` either read via telemetry or deleted).
2. No unreferenced helpers (`sleep_until_us` gone).
3. No duplicated derivation between engine and telemetry (lateness counters single-sourced).
4. `play()` reads as wait → drain → observe with a typed `LoopState`; no 3-tuple threading.
5. Backend state-tracking lives in one place.
6. Late-burst behavior is defined, observable, off-by-default, and documented.
7. Anchor correction validated in-game at `local_precise @144fps`.
8. Full suite green and audit bench invariants (drift 0, drops 0, shortfall 0) hold at every phase.

## Recommended order

```text
Phase A (dead code + telemetry wiring)
-> Phase B (telemetry de-dup)
-> Phase C (play() refactor, equivalence-gated)
-> Phase D (backend de-dup)
-> Phase E (focus-burst decision, comment only)
-> Phase F (late-burst policy, off by default)
-> Phase G (in-game validation) -> only then any margin decision
```

Phases A, B, D, E are low-risk and can be batched. Phase C must be equivalence-gated. Phase F must
not ship enabled before Phase G.
