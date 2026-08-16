# 09 — Implementation Sequence for the Coding Agent

The coding agent must execute in this order. Do not combine phases just because adjacent files overlap.

Each phase must leave the repository buildable/testable. Temporary compatibility wrappers are allowed only when explicitly described and must be removed by the listed cleanup phase.

---

# Phase 0 — Capture implementation baseline

## Purpose

Prevent performance claims from drifting as `main` changes.

## Actions

1. Record current implementation-base SHA.
2. Run repository gates from `08_TEST_AND_BENCHMARK_MATRIX.md`.
3. Run existing `rt_handoff_bench` and native acceptance benchmark.
4. Capture Windows environment metadata.
5. Capture old production effective spin behavior.

## No code semantics changes

Do not “fix a small thing while baselining”.

## Exit condition

Reproducible baseline data exists for the branch that implementation will modify.

---

# Phase 1 — Native deterministic hold-feasibility validator

## Scope

```text
sky_dispatch_core schedule validator
PyO3/native session construction
unit/property/Python boundary tests
hold documentation mismatch
```

## Required implementation

- add structured same-key hold validation in `sky_dispatch_core`;
- validate against final `effective_min_hold_us` before worker start;
- preserve overflow validation;
- expose precise native construction error;
- no worker/hot-path changes yet.

## Why first

It removes deterministic impossible schedules before the more complex runtime release refactor and gives later phases a stronger input contract.

## Exit condition

All `<`, `==`, `>` min-hold boundary tests pass and direct native callers cannot bypass validation.

---

# Phase 2 — Core per-key pending-release model

## Scope

`sky_dispatch_core` only as far as practical.

## Required implementation

- add fixed `[Option<PendingRelease>; MAX_KEYS]` + mask;
- implement authored-frame preparation classification:
  - immediate Up;
  - deferred Up;
  - stale Up;
  - atomic Down;
- implement dynamic same-key infeasibility error;
- implement metadata-only frame commit;
- implement pending-release commit/query methods;
- full core unit/property tests.

## Migration constraint

Production player may temporarily still use the old coordinator API during the first commit of this phase only if necessary to keep changes reviewable. A compatibility wrapper must be clearly marked temporary and must not duplicate mutable state.

By phase exit, core tests/simulation should exercise the new semantic model.

## Do not yet optimize hot path

No queue/QPC/spin cleanup in this phase.

---

# Phase 3 — New boundary selector and typed plan

## Scope

Coordinator/player planning boundary.

## Required implementation

- planner selects min(next authored frame, earliest pending release);
- equal boundary coalescing;
- introduce typed `NextDispatchPlan` variants;
- physical variant owns exact QPC target + prepared packet + commit token;
- metadata variant owns exact bounded metadata commit token;
- remove health budget from physical validity;
- build `PreparedPhysicalPacket` before wait;
- add mock/test-support plan-selection tests.

## Must prove

```text
planner target is unique
Down chord is never split
unrelated deferred Up cannot move Down target
```

## No production old/new hybrid state machine

At phase exit there must be one planner model. Do not keep a hidden “legacy packet deadline” path selected by flags.

---

# Phase 4 — Wire production dispatch to per-key release semantics

## Scope

`sky_player_rs` authored/pending dispatch + Win32 prepared packet call.

## Required implementation

- dispatch all new physical plan variants;
- commit exact frozen token on full success;
- metadata-only frame commit without SendInput;
- one authored physical SendInput attempt;
- structural failure on dynamic same-key infeasibility;
- remove packet-wide release floor as production target source;
- maintain cleanup behavior on any ambiguous transport result.

## Required fault tests

DownOnly, UpOnly pending, Mixed, coalesced, all partial prefixes.

## Performance requirement

No heap allocation introduced in new precision paths.

## Exit condition

P0 Case A–J in `02_P0_RELEASE_SCHEDULER_AND_FEASIBILITY.md` pass through production-style harness.

---

# Phase 5 — Add no-catch-up backlog guard

## Scope

Worker runtime/dispatch loop only.

## Required implementation

- add explicit `awaiting_future_physical_boundary` state;
- set after successful physical send;
- clear only after a selected future physical target is reached through deadline wait;
- reject second already-due physical send;
- metadata boundaries cannot clear it;
- structural fatal independent of strict profile;
- add deterministic stall tests.

## Do not add

- timeline rebase;
- event dropping policy;
- arbitrary production lateness threshold;
- adaptive catch-up spacing.

## Exit condition

Test invariant: no more than one physical call after a stall before a new future-deadline wait.

---

# Phase 6 — Unify target ownership and remove redundant QPC handoff work

## Scope

planning/wait/dispatch handoff.

## Required implementation

- wait accepts frozen `physical_target_qpc`;
- remove production target reconstruction from epoch + logical deadline;
- `WaitBoundary::Due` refers to same target;
- reuse `WaitResult::wake_qpc` after deadline;
- remove immediate post-wake QPC sample used only for effective-now/telemetry;
- preserve lease-only bounded wake/replan semantics;
- target-identity and QPC-read-count tests.

## Performance A/B

Run handoff benchmark before/after.

## Exit condition

Every physical plan has exactly one target calculation and no target drift across planner/wait/dispatch.

---

# Phase 7 — Minimize post-SendInput producer work

## Scope

coordinator commit, observation producer, observer materialization.

## Required implementation

- release build commit validates touched slots only;
- keep full invariant scanner in tests/debug/finalization;
- create compact raw dispatch observation;
- move derived timing/health/compatibility work to observer;
- remove producer queue `len()`/exact high watermark;
- preserve nonblocking drop-new behavior;
- keep ArrayQueue;
- expand no-allocation tests.

## Diagnostic strict profile

Keep only the minimum completion-late check required to terminate strict diagnostic mode before a later send. Format strings only on actual terminal result.

## Performance objective

Measure `completion_to_rt_ready` and satisfy `08_TEST_AND_BENCHMARK_MATRIX.md`.

---

# Phase 8 — Deterministic Windows wait policy

## Scope

Win32 waiter/startup/config.

## Required implementation

- fixed 700 µs production spin threshold;
- startup wake probe no longer controls production threshold;
- mandatory production high-res timer + event constructor;
- remove production `timeBeginPeriod` fallback/guard;
- keep benchmark/test wait variants separately;
- keep MMCSS Games/High and HighQoS;
- no TimeCritical/affinity changes.

## A/B

Run fixed-threshold matrix and old-policy baseline.

## Exit condition

Production threshold is deterministic and Windows waiter requirements are explicit in types/startup errors.

---

# Phase 9 — Timing semantic cleanup

## Scope

Rust names, observer records, Python/report docs/stubs.

## Required implementation

- use `final_admission_qpc` internally;
- map compatibility `send_started_ticks` alias truthfully;
- preserve `sendinput_completion_qpc` semantics;
- diagnostic-only optional syscall-entry sample if benchmark harness needs it;
- remove misleading onset/actual wording where applicable;
- signed residual tests;
- no controller feedback.

## Do not force schema churn

Prefer compatibility aliases unless ambiguity cannot be repaired without a schema change.

---

# Phase 10 — Dead compatibility/control cleanup

## Scope

P2 items in `07_P2_COMPATIBILITY_AND_DEAD_CODE.md`.

## Required implementation

- remove/move dead adaptive spin controller code;
- isolate test-only authored retry policy and legacy retry semantics;
- remove old plan/health-budget scaffolding;
- remove old packet-wide deadline helpers;
- trim timeline-rebase metrics from producer observation;
- simplify wait options;
- preserve stable public compatibility values where required.

## Rule

Every deletion must be supported by repository-wide search and tests. Do not delete a diagnostic tool because production does not use it.

---

# Phase 11 — Normative documentation and final Windows acceptance

## Update

```text
docs/timing-principles.md
docs/hold-frame-model.md
docs/rt-dispatch-architecture.md
relevant architecture/API docs
Python type stubs/golden schemas if needed
```

## Run

Full gate + real Windows acceptance matrix + soak test.

## Final review questions

1. Can any unrelated release move a Down chord? Must be no.
2. Can any authored Down chord be split? Must be no.
3. Can two overdue distinct physical boundaries be fired back-to-back? Must be no.
4. Can wait compute a different physical target from planner? Must be no.
5. Can observer/health state affect target/send authorization? Must be no.
6. Does production adapt spin or note lead from startup/history? Must be no.
7. Does healthy producer allocate/block/full-scan? Must be no.
8. Are all structural faults fail-closed with cleanup? Must be yes.
9. Are Windows performance claims backed by real-host A/B? Must be yes.

---

# Commit/PR discipline

Preferred: one PR per numbered phase above; Phase 2/3 may be split into small mechanical + semantic commits inside their PR if needed.

Do not combine:

- P0 release semantics with wait tuning;
- stall policy with telemetry cleanup;
- QPC boundary naming with scheduler compensation experiments;
- hot-path cleanup with broad formatting/module moves.

For any phase touching the hot path, PR description must contain:

```text
Before behavior
After behavior
Invariant protected
Benchmark command
Before result
After result
Failure/rollback condition
```

---

# Stop conditions for the coding agent

Stop the current phase and report rather than redesign when:

- a required invariant is mutually incompatible with another locked decision;
- a public API cannot be preserved without a deliberate schema decision not covered here;
- real Windows A/B violates the acceptance threshold after the intended optimization;
- cleanup cannot prove all keys released after a new failure path;
- implementation would require a new input mechanism, hook, driver, or game-side integration.

The coding agent may propose evidence, but it may not silently change this plan.
