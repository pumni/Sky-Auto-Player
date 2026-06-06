> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: nhắc tới cơ chế start-anchor cũ và các tham số timing đã gỡ.

# Runtime Hold Enforcement Refactor Plan

Date: 2026-06-05

Status: Phases 0-6 implemented on 2026-06-05. Phase 6 late-burst policy is implemented
off-by-default via `late_pulse_drop_threshold_us`.

> **Superseded anchor note (2026-06-06).** The later approved completion-anchor refactor supersedes
> the post-Phase-5 start-anchor correction. Runtime now anchors the hold floor to down dispatch
> **completion**, with scheduler same-key feasibility guarded by `release_latency_margin_us`.
> Read `docs/completion-anchor-refactor-plan.md` and `docs/timing-principles.md` §7 as the current
> contract. Historical sections below are retained for audit context.

Related:

- `down-hold-up-scheduling-audit.md`
- `scheduler-core-architecture-plan.md`
- `timing-principles.md`
- `timing-experiments.md`

## 0. Decision

Refactor runtime dispatch before changing frame rounding or increasing profile `min_hold`.

The target is:

```text
preserve authored down onset
+ enforce min_hold from confirmed down dispatch
+ defer release per key without blocking unrelated downs
+ report exactly what SendInput accepted or skipped
```

The current pure scheduler remains the source of the authored absolute timeline. This refactor does
not replace it and does not change profile values.

## 1. Scope

### In scope

- Runtime correlation of each scheduled down/up pair.
- Per-key active generation tracking.
- Truthful backend-result telemetry.
- A non-blocking per-key release eligibility guard.
- Explicit handling of runtime same-key conflicts.
- Correct state reset after pause, panic, focus loss, and emergency release.
- Deterministic tests for asymmetric lateness and deferred releases.
- Real SendInput telemetry and in-game validation before any margin decision.

### Out of scope

- Changing `ceil()` or frame materialisation.
- Increasing any built-in `min_hold_frames`.
- Reintroducing a general release gap or repeat-gap profile field.
- Modifying game files, reading game memory, or using anything other than Windows SendInput.
- Rewriting the pure domain scheduler.
- Changing authored down timestamps.
- Changing normal CLI options or defaults.
- Solving game-side frame-time variance before sender/runtime correctness is enforced.

## 2. Frozen behavior during the refactor

The following behaviors are frozen until the final experiment phase:

1. `build_key_actions()` remains pure and backend-independent.
2. `KeyAction.at_us` remains the authored absolute schedule.
3. Exact simultaneous chords remain batched.
4. Normal downs remain onset-priority events.
5. Built-in policies continue to materialise their current values.
6. Backend duplicate-down protection remains as a final safety boundary.
7. Degraded runtime conflict policy initially preserves the currently active note and drops the
   conflicting new down.
8. Strict preflight behavior remains unchanged until runtime strict-conflict handling is added.
9. Existing CSV fields and summary keys remain readable; telemetry changes are additive.
10. Golden scheduler snapshots must remain unchanged.

The degraded choice in item 7 is intentionally conservative. Prioritising a new onset would require
releasing the active note early and is a separate musical-policy decision.

## 3. Target runtime contracts

### 3.1 Authored schedule contract

The domain scheduler answers:

```text
What down/up state transitions are requested, and at what musical timestamps?
```

It does not claim that every requested transition was successfully sent or that every requested
hold survived runtime lateness.

### 3.2 Runtime dispatch contract

The runtime executor answers:

```text
Which requested transitions were sent, deferred, dropped, skipped, or cancelled?
```

For every successfully sent down generation, a normal successfully sent up must satisfy:

```text
up_dispatch_started_us >= down_dispatch_completed_us + min_hold_us
```

Exceptions must be explicitly classified:

- `emergency_release`
- `pause_release`
- `focus_loss_release`
- `panic_release`
- `runtime_conflict_drop`
- `backend_skip`
- `backend_failure`

### 3.3 Backend contract

The backend remains a thin SendInput boundary:

- deduplicate scan codes inside a batch;
- reject duplicate downs as a safety net;
- make key-up idempotent;
- return `InputSendResult`;
- maintain panic-release health state.

The backend must not silently become the musical conflict-policy engine.

### 3.4 Telemetry contract

Telemetry distinguishes:

- scheduled intent;
- dispatch attempt;
- actually sent scan codes;
- skipped scan codes;
- deferred releases;
- dropped generations;
- confirmed hold lower bound.

An attempted down that the backend skipped must never be counted as a sent note.

## 4. Why runtime generations are required

`KeyAction` currently has no note identity. That becomes unsafe when a same-key down is dropped or a
key is released by pause/focus handling.

Example:

```text
down generation 1
down generation 2 -> dropped because key is active
up generation 1
down generation 3
up generation 2 -> must NOT release generation 3
```

Without generation correlation, the last up can release the wrong active note.

The scheduler output should remain stable. Therefore generation identity will be added by a pure
runtime compilation step rather than by changing `KeyAction`.

## 5. Proposed runtime data model

Create `src/sky_music/orchestration/runtime_dispatch.py`.

The exact names may change during implementation, but the responsibilities must remain separate.

```python
from dataclasses import dataclass
from typing import Literal


GenerationStatus = Literal[
    "scheduled",
    "active",
    "release_pending",
    "released",
    "dropped_conflict",
    "dropped_backend",
    "cancelled",
]


@dataclass(frozen=True, slots=True)
class RuntimeKeyIntent:
    source_action_index: int
    batch_id: int
    generation_id: int | None
    kind: Literal["down", "up"]
    scan_code: int
    scheduled_us: int
    reason: str


@dataclass(frozen=True, slots=True)
class ActiveKeyGeneration:
    generation_id: int
    scan_code: int
    source_action_index: int
    scheduled_down_us: int
    down_dispatch_started_us: int
    down_dispatch_completed_us: int
    release_not_before_us: int


@dataclass(frozen=True, slots=True)
class PendingRelease:
    generation_id: int
    scan_code: int
    source_action_index: int
    scheduled_release_us: int
    release_not_before_us: int
    reason: str

    @property
    def effective_release_us(self) -> int:
        return max(self.scheduled_release_us, self.release_not_before_us)
```

Runtime state is owned by one `PlaybackEngine` instance. No globals are introduced.

The active-key state may use a small mutable coordinator internally, but all decision inputs and
outputs should be typed and directly unit-testable.

## 6. Runtime schedule compilation

Add a pure function:

```python
compile_runtime_intents(actions: tuple[KeyAction, ...]) -> RuntimeSchedule
```

### Required behavior

1. Explode each batched `KeyAction` into one `RuntimeKeyIntent` per scan code.
2. Preserve `source_action_index` and a stable `batch_id`.
3. Assign a monotonically increasing `generation_id` to every down intent.
4. Pair up intents to same-key down generations using FIFO order.
5. Keep unpaired ups explicit with `generation_id=None`.
6. Preserve the existing up-before-down order at the same timestamp.
7. Do not alter timestamps.

FIFO pairing matches the current scheduler's per-key lane ordering and handles overlapping degraded
schedules:

```text
down g1, down g2, up g1, up g2
```

### Equivalence requirement

If runtime intents are regrouped by original `batch_id`, they must reproduce the original
`KeyAction` timeline exactly.

This requirement is the first protection against accidentally changing scheduler semantics.

## 7. Runtime dispatch state machine

### 7.1 State

The runtime coordinator owns:

- cursor into the compiled authored intents;
- active generation by scan code;
- generation status by generation ID;
- pending release by generation ID;
- runtime diagnostics and counters;
- monotonic dispatch ID.

At most 15 musical keys are active, so correctness and clarity are more important than complex data
structures. A heap for pending releases is acceptable, but a small scanned collection is also
sufficient if it keeps the implementation simpler and deterministic.

### 7.2 Next deadline

The engine must wait for:

```text
min(next authored action timestamp, earliest pending effective release)
```

This lets a deferred release fire before the next authored action without blocking unrelated
events.

### 7.3 Same-deadline priority

At a shared deadline, process:

1. previously deferred releases that are now eligible;
2. authored up intents;
3. newly eligible releases created by those up intents;
4. authored down intents.

This preserves current up-before-down semantics and ensures an eligible same-key release happens
before the next down.

### 7.4 Down handling

For each due authored down intent:

- if its scan code has no active generation, include it in the down dispatch batch;
- if its scan code is active:
  - degraded: mark the new generation `dropped_conflict`;
  - strict runtime mode: abort playback after emergency release;
- dispatch all remaining chord scan codes in one backend call;
- use `InputSendResult.sent` to activate only scan codes actually accepted by the backend;
- mark unexpected backend duplicates as `dropped_backend`.

Dropping one conflicting chord key must not drop the other playable chord keys.

### 7.5 Down result handling

For each successfully sent down:

```text
release_not_before_us = down_dispatch_completed_us + min_hold_us
```

Use playback elapsed time, not raw system time, so the value remains in the same clock domain as
scheduled actions and telemetry.

Using dispatch completion is intentionally conservative. SendInput has confirmed insertion before
the call returns; a later up call cannot begin before that completion point.

### 7.6 Up handling

For each authored up intent:

- if its generation was dropped or cancelled, consume it without calling the backend;
- if it is unpaired, record `unpaired_up`;
- if it matches the currently active generation:
  - create a `PendingRelease`;
  - send immediately if `effective_release_us <= now_us`;
  - otherwise defer it;
- if it does not match the active generation, classify it as stale and do not release another
  generation.

For each release backend result:

- mark only `sent` generations released;
- classify skipped keys explicitly;
- retain or emergency-release failed keys according to backend health behavior.

### 7.7 Partial up batches

A scheduled up batch may contain keys with different runtime eligibility.

The coordinator must:

- send the eligible subset now;
- defer the ineligible subset independently;
- preserve source-action attribution for both subsets.

This is required for chords and for batches whose keys had different SendInput completion times.

### 7.8 End of playback

Playback is complete only when:

```text
authored intent cursor exhausted
AND pending release set empty
```

Then the existing final `release_all()` remains as a safety net.

The renderer may reach nominal 100% before a short deferred release drains. It must not print
`Finished` until the runtime queue is empty.

## 8. Pause, focus, panic, and emergency release

All non-musical `release_all()` calls must go through one engine helper:

```python
_release_all_and_cancel_runtime(reason: RuntimeCancelReason)
```

This helper:

1. calls backend `release_all()`;
2. marks active generations cancelled;
3. removes their pending releases;
4. ensures their future authored ups are suppressed by generation status;
5. records the cancellation reason.

Required reasons:

- `manual_pause`
- `focus_loss`
- `panic`
- `quit`
- `skip`
- `exception`
- `playback_finalizer`

Safety releases are allowed to violate `min_hold`; telemetry must exclude them from normal hold-floor
compliance metrics.

## 9. Execution and telemetry types

### 9.1 Expand dispatch result

Evolve `ExecutionResult` or replace it with a clearly named internal `DispatchResult`.

Required fields:

```python
dispatch_id: int
source_action_indices: tuple[int, ...]
generation_ids: tuple[int, ...]
kind: Literal["down", "up"]
scheduled_us: int
dispatch_started_us: int
dispatch_completed_us: int
lateness_us: int
send_duration_us: int
requested_scan_codes: tuple[int, ...]
sent_scan_codes: tuple[int, ...]
skipped_scan_codes: tuple[int, ...]
reason: str
runtime_outcome: str
```

Retain compatibility accessors or fields for `event_index` and `actual_us` during migration if this
keeps existing tests and report readers stable.

### 9.2 CSV schema

Keep current fields and add:

- `dispatch_id`
- `dispatch_completed_us`
- `sent_scan_codes`
- `skipped_scan_codes`
- `generation_ids`
- `runtime_outcome`
- `deferred_by_us`
- `confirmed_hold_lower_bound_us`

`event_index` continues to identify the original scheduled action where possible. A split/deferred
dispatch may produce multiple rows sharing one `event_index`; `dispatch_id` is unique.

### 9.3 Summary schema

Add:

- `attempted_dispatches`
- `successful_dispatches`
- `sent_down_count`
- `sent_up_count`
- `backend_skipped_down_count`
- `backend_skipped_up_count`
- `runtime_conflict_dropped_down_count`
- `cancelled_generation_count`
- `deferred_release_count`
- `release_deferral_us`
- `confirmed_hold_lower_bound_us`
- `confirmed_hold_shortfall_count`
- `runtime_same_key_up_gap_us`

Keep old `note_hold_duration_us` as a compatibility metric, but document it as dispatch-start to
dispatch-start. The new confirmed lower-bound metric governs runtime hold validation.

### 9.4 Calibration compatibility

Calibration must continue reading old summaries.

New summary fields are advisory during this refactor. Do not automatically raise profile hold values
from runtime shortfall data while the guard is being introduced.

## 10. Implementation phases

Each phase must land with a green full test suite. Do not combine the phases into one broad rewrite.

### Phase 0 - Baseline freeze and fixtures

Goal: pin current behavior before changing runtime execution.

Changes:

- Add deterministic fixtures for:
  - normal single note;
  - exact chord;
  - same-key interval below, equal to, and above `min_hold`;
  - three-generation overlap;
  - pause during hold;
  - asymmetric down/up lateness;
  - unrelated down occurring while another key release is deferred.
- Add a helper that captures backend dispatch batches with timestamps.
- Record current golden scheduler output without regenerating it.

Gates:

- `uv run pytest`
- golden scheduler snapshots unchanged;
- corpus schedule actions unchanged at 30/60/144 FPS.

No production behavior change.

### Phase 1 - Truthful backend-result telemetry

Goal: stop reporting attempted actions as successfully sent actions.

Changes:

- Capture `InputSendResult` in `PlaybackEngine._execute_action()`.
- Add dispatch completion time to execution results.
- Add sent/skipped scan codes to telemetry.
- Update summary calculation to pair only actually sent downs and ups.
- Add compatibility parsing for old summaries.

Tests:

- backend-skipped duplicate down is recorded as skipped, not sent;
- idempotent up is recorded as skipped;
- partial chord send reports sent and skipped subsets correctly;
- old telemetry summary fixtures still load;
- normal existing playback history is unchanged.

Gates:

- no timing behavior change;
- existing CSV columns retained;
- full suite green.

### Phase 2 - Pure runtime intent compiler

Goal: introduce generation identity without changing dispatch.

Changes:

- Add `runtime_dispatch.py` types and `compile_runtime_intents()`.
- Add FIFO down/up generation pairing.
- Add compile diagnostics for unpaired ups.
- Keep engine on the old action loop.

Tests:

- compile/regroup equivalence for all golden schedules;
- chords preserve batch identity;
- overlap sequence pairs `down g1, down g2, up g1, up g2`;
- same-timestamp up-before-down pairs correctly;
- unpaired up remains explicit;
- corpus compile succeeds for every parsed song.

Gates:

- zero scheduler/golden changes;
- zero production dispatch changes;
- full suite green.

### Phase 3 - Runtime coordinator in equivalence mode

Goal: replace the sequential action iterator with the runtime coordinator while hold guard is
disabled.

Changes:

- Add authored-intent cursor, generation status, active-key ledger, and pending-release support.
- Set `runtime_min_hold_us=0` in equivalence mode.
- Regroup due intents into backend batches.
- Synchronise pause/focus/panic release paths with runtime state.
- Drain runtime queue before finishing.

Tests:

- old engine and coordinator produce identical backend history for all feasible golden schedules;
- chord batching remains identical;
- authored downs retain exact scheduled deadlines under fake time;
- pause/focus/panic do not allow stale ups to release a later generation;
- dropped-generation ups are suppressed;
- partial chord conflicts preserve playable keys;
- finalizer releases all state.

Gates:

- deterministic dispatch equivalence when guard is disabled;
- down lateness benchmark does not regress;
- full suite green.

Rollback boundary:

- engine can temporarily switch back to the old iterator because no profile or scheduler change has
  occurred.

### Phase 4 - Enable per-key runtime hold guard

Goal: enforce the existing `min_hold_us` from confirmed down dispatch.

Changes:

- Pass resolved `active_policy.min_hold_us` from `main.py` to `PlaybackEngine`.
- Calculate `release_not_before_us` from successful down completion.
- Defer ineligible releases per key.
- Wake on the earliest pending release or authored action.
- Keep unrelated downs dispatchable.
- Record release deferral and confirmed hold lower bounds.

Tests:

- asymmetric lateness cannot shorten confirmed hold below `min_hold_us`;
- release occurs at exactly `max(scheduled_up, down_completed + min_hold)`;
- unrelated down remains on time while another key release is deferred;
- partial chord release splits correctly;
- multiple simultaneous eligible releases are batched;
- exact-boundary same-key down becomes an explicit conflict, not an accidental backend duplicate;
- safety release bypasses the floor and is excluded from compliance metrics;
- playback waits for final deferred release.

Hard gates:

- every normal sent up has confirmed hold lower bound `>= min_hold_us`;
- no unrelated down is shifted by release deferral in deterministic tests;
- normal real-song corpus at `1.0x` produces no runtime conflict drops;
- full suite green.

### Phase 5 - Explicit runtime conflict policy

Goal: remove accidental reliance on backend duplicate filtering.

Initial degraded policy:

```text
preserve active generation
drop conflicting new down generation
continue other chord keys
```

Changes:

- classify runtime conflicts before backend call;
- suppress the future up belonging to a dropped generation;
- add runtime strict conflict abort;
- expose runtime conflict counts in telemetry and HUD/reporting;
- update warning text to distinguish schedule infeasibility from runtime conflict caused by confirmed
  hold enforcement.

Tests:

- three-generation overlap never lets stale up release a later generation;
- degraded conflict drop is deterministic and truthful;
- strict conflict aborts and releases all keys;
- mixed chord conflict sends nonconflicting keys;
- backend duplicate-down skip remains zero for planner-known conflicts.

Gates:

- backend duplicate filtering is only a safety net in normal test paths;
- full suite green.

### Phase 6 - Late-burst recovery policy

Goal: avoid replaying expired pulses back-to-back after a long thread stall.

Rejected decision: rebasing the timeline after a stall causes permanent and cumulative musical
slowdown. Runtime stalls must never modify the absolute music clock. Only explicit user pause and
focus-loss pause may shift the timeline.

Any chosen policy must:

- preserve safety;
- never fabricate extra downs;
- report dropped/expired generations;
- preserve the absolute authored timeline without cumulative drift.

### Phase 7 - In-game validation and margin decision

Goal: determine whether the game itself needs margin after runtime correctness is enforced.

Keep profile ratios unchanged during Phases 0-6.

Run:

1. one-frame visibility tests at 30/60/144 FPS;
2. steady alternate-key timing test;
3. same-key boundary tests;
4. real-song local playback;
5. optional remote audience validation.

Compare baseline and guarded runtime:

- sent down onset IOI;
- down lateness p50/p95/p99/max;
- release deferral distribution;
- confirmed hold lower-bound distribution;
- backend skips;
- runtime conflict drops;
- in-game onset registration.

Only after this phase decide whether to:

- keep `local_precise = 1.0 frame`;
- add a small measured game-side margin;
- or change another profile ratio.

## 11. Detailed test matrix

### Runtime compiler

| Case | Expected |
| --- | --- |
| one down/up | one generation paired |
| exact chord | separate generations, shared batch |
| same-key overlap | FIFO generation pairing |
| same timestamp up/down | old generation up pairs before new down |
| unpaired up | explicit diagnostic, no invented generation |

### Release guard

| Case | Expected |
| --- | --- |
| down on time, up on time | up deferred by down SendInput duration |
| down late, up nominal | up deferred enough to preserve floor |
| up already later than floor | no extra deferral |
| chord downs share one send | each generation uses shared completion time |
| partial release eligibility | eligible subset sent, rest deferred |
| unrelated down during deferral | unrelated down remains on authored deadline |

### Same-key conflicts

| Case | Expected |
| --- | --- |
| interval below runtime floor | new down explicitly dropped in degraded mode |
| interval equal runtime floor | release first if eligible; otherwise explicit conflict |
| dropped generation later up | suppressed |
| three overlapping generations | no stale up releases later active generation |
| mixed chord with one conflict | other chord keys still sent |

### Lifecycle

| Case | Expected |
| --- | --- |
| pause during hold | release_all, generation cancelled, later up suppressed |
| focus loss during hold | same as pause |
| panic | immediate release, all runtime state cancelled |
| quit/skip | no pending release survives finalizer |
| exception in backend | emergency cleanup and truthful failure telemetry |

### Telemetry

| Case | Expected |
| --- | --- |
| backend skips down | sent count unchanged, skipped count increments |
| deferred release | two timestamps and deferral recorded |
| normal sent pair | confirmed lower-bound hold calculated |
| safety release | excluded from normal floor compliance |
| old summary JSON | still readable |

## 12. Performance and timing gates

### Deterministic gates

- No unrelated authored down moves because of another key's deferred release.
- Coordinator equivalence mode produces identical backend batches to the old engine for feasible
  schedules.
- Runtime intent compilation and decision logic perform no OS calls.
- Final spin remains free of focus checks, controls polling, rendering, and telemetry file I/O.

### Real-machine benchmark

Use the existing randomized O10.5 method with at least 7 runs per condition.

Conditions:

- baseline engine;
- coordinator equivalence mode;
- coordinator with runtime hold guard.

Songs:

- `TEST_metro_alt_120`
- `TEST_metro_alt_200`
- one chord-heavy test
- one normal real song

Provisional acceptance:

- candidate median down-lateness p95 no worse than baseline by more than 0.1 ms;
- no new systematic down IOI bias;
- no new down events above 2 ms in clean runs;
- no backend duplicate skips in normal feasible songs;
- confirmed hold shortfall count is zero with guard enabled.

If a timing gate fails, optimize the coordinator before changing profile values.

## 13. File-by-file change map

### New

- `src/sky_music/orchestration/runtime_dispatch.py`
  - runtime intent compiler;
  - generation pairing;
  - per-key state machine;
  - next-deadline and due-dispatch decisions.

- `tests/test_runtime_dispatch.py`
  - pure compiler and state-machine tests;
  - deterministic deferred-release tests.

### Modify

- `src/sky_music/orchestration/engine.py`
  - capture backend results;
  - integrate runtime coordinator;
  - wait on authored/pending deadlines;
  - centralise release-all state cancellation.

- `src/sky_music/orchestration/telemetry.py`
  - additive dispatch/result fields;
  - confirmed hold lower-bound metrics;
  - sent/skipped/drop summaries;
  - old-summary compatibility.

- `src/sky_music/infrastructure/backend.py`
  - keep behavior stable;
  - clarify `InputSendResult` contract;
  - only change if additional failure attribution is strictly necessary.

- `src/main.py`
  - pass resolved `min_hold_us` and conflict policy to engine;
  - preserve CLI behavior.

- `tests/test_playback.py`
  - event-loop and lifecycle integration.

- `tests/test_engine_refactor.py`
  - expanded dispatch result and telemetry compatibility.

- `tests/test_acceptance_flow.py`
  - end-to-end policy propagation and no-conflict baseline.

- `docs/architecture.md`
  - document authored scheduler versus runtime enforcement.

- `docs/timing-principles.md`
  - distinguish scheduled hold from confirmed runtime hold.

- `docs/timing-experiments.md`
  - add guarded-runtime validation procedure.

### Must remain unchanged through Phase 5

- scheduler golden snapshots;
- built-in profile frame ratios;
- frame rounding formula;
- public CLI flags and defaults;
- Windows SendInput-only backend boundary.

## 14. Risks and mitigations

### Risk: runtime guard delays musical downs

Mitigation:

- per-key deferred release queue;
- unrelated downs remain independent;
- deterministic no-shift tests;
- real-machine down-lateness gate.

### Risk: stale up releases a later note

Mitigation:

- runtime generation IDs;
- FIFO pairing;
- generation status checks before every up;
- three-generation overlap regression test.

### Risk: chord batching regresses

Mitigation:

- preserve original `batch_id`;
- regroup playable due downs;
- partial-split tests only where runtime eligibility differs.

### Risk: pause/focus paths desynchronise backend and runtime

Mitigation:

- one `_release_all_and_cancel_runtime()` helper;
- cancellation status suppresses later ups;
- lifecycle test matrix.

### Risk: telemetry format breaks calibration

Mitigation:

- additive columns and keys;
- retain old compatibility metrics;
- old-summary fixtures;
- defer calibration behavior changes.

### Risk: confirmed-completion guard systematically extends notes

Mitigation:

- extension is adaptive and measured;
- no profile margin increase during refactor;
- record deferral distribution;
- validate articulation in-game before accepting.

### Risk: exact-boundary repeats drop more often

Mitigation:

- this exposes a real physical conflict that current scheduling hides;
- normal corpus at `1.0x` has ample same-key headroom;
- report conflicts truthfully;
- keep profile/tempo decision separate from runtime correctness.

## 15. Rollback strategy

Keep rollback boundaries between phases:

1. Phase 1 telemetry can be reverted without timing changes.
2. Phase 2 compiler is unused by production and can be removed independently.
3. Phase 3 coordinator must support equivalence mode before guard activation.
4. Phase 4 guard activation is isolated to engine policy propagation and coordinator eligibility.
5. Profile values remain untouched, so no config migration rollback is required.

Do not remove the old action-loop implementation until Phase 4 passes deterministic, corpus, and
real-machine gates. It may remain temporarily as a private fallback during the staged refactor, but
must not become a permanent user-facing timing mode.

## 16. Definition of done

The refactor is complete when:

1. Every normal sent up satisfies:

   ```text
   up_dispatch_started >= down_dispatch_started + min_hold
   ```

2. No unrelated down is delayed by a deferred release.
3. No stale or dropped-generation up can release a later generation.
4. Telemetry distinguishes attempted, sent, skipped, deferred, dropped, and cancelled actions.
5. Backend duplicate-down filtering is only a safety net for unexpected state drift.
6. Pause, focus loss, panic, skip, quit, and exceptions leave no active runtime generation.
7. Scheduler golden outputs and CLI behavior remain unchanged.
8. Full tests and real-machine timing gates pass.
9. In-game one-frame validation is rerun with the guarded runtime.
10. Frame margin is reconsidered only from post-refactor evidence.

## 17. Recommended implementation order

Proceed in this order:

```text
Phase 0 fixtures
-> Phase 1 truthful telemetry
-> Phase 2 runtime generation compiler
-> Phase 3 coordinator equivalence mode
-> Phase 4 per-key hold guard
-> Phase 5 explicit conflict policy
-> real-machine and in-game validation
-> only then margin/rounding decision
```

The first production timing behavior change should occur only in Phase 4, after observability,
generation identity, and equivalence tests are already in place.
