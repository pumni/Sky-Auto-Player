# 12 — Coding Agent Handoff / Execution Contract

Use this file as the entrypoint when handing implementation to an AI coding agent.

---

## Mandatory first instruction

Before changing code, read **every file** in:

```text
docs/plan/2026-08-16_rt-dispatch-round2-authoritative/
```

in numeric order, starting with `README.md` and `00_DECISION_REGISTER.md`.

This folder is an authoritative implementation specification. You are not being asked to redesign the system. Architecture decisions have already been made.

---

# Role

You are the implementation agent.

Your responsibilities are:

1. implement the decisions exactly;
2. write/update the required tests;
3. run the required gates/benchmarks;
4. report evidence and any contradiction.

Your responsibilities do **not** include choosing a different timing policy, recovery policy, packet semantic, scheduler controller, queue architecture, or Windows-priority strategy.

---

# Baseline

The architecture audit that produced this plan used:

```text
pumni/Sky-Auto-Player
main@bd5542dd01b6612eea0ebb48c0f6e7a27d8e690e
```

Before implementation, record the actual implementation-base SHA. If it differs, inspect the diff for files touched by `10_FILES_TOUCH_MAP.md` and report any semantic conflict before proceeding.

Do not assume a later `main` still has exactly the audited behavior.

---

# Execution order

Follow `09_IMPLEMENTATION_SEQUENCE.md` exactly:

```text
Phase 0  baseline
Phase 1  native min-hold feasibility
Phase 2  core pending-release model
Phase 3  boundary selector + typed plan
Phase 4  production per-key release dispatch
Phase 5  overdue backlog guard
Phase 6  single QPC target + handoff cleanup
Phase 7  post-SendInput/observer minimization
Phase 8  deterministic Windows wait policy
Phase 9  timing semantics cleanup
Phase 10 dead compatibility/control cleanup
Phase 11 normative docs + final Windows acceptance
```

Do not jump to P1 micro-optimizations before P0 semantics pass.

---

# Non-negotiable implementation outcomes

At the end:

```text
1. Down chord keys are never split across physical sends.
2. A delayed Up for unrelated key cannot move a Down chord.
3. A same-key release that cannot meet the Down chord target causes fail-closed termination, not chord delay/split.
4. Authored same-key hold below effective native floor is rejected before worker start.
5. Multiple overdue physical boundaries are never drained as a catch-up burst.
6. Planner/wait/dispatch share one exact frozen physical_target_qpc.
7. Production keeps one final_admission_qpc sample and one completion sample around the sender envelope; no cosmetic extra QPC read.
8. No learned dispatch lead or adaptive note-on compensation exists.
9. Production spin is fixed 700 µs.
10. Production requires high-resolution waitable timer + event and does not depend on timeBeginPeriod.
11. MMCSS Games/High + HighQoS remain; TimeCritical/realtime/affinity are not defaults.
12. Healthy RT producer allocates nothing, blocks on no mutex, performs no full-state scan, and does one nonblocking observation enqueue.
13. Partial/ambiguous Down-bearing SendInput is terminal and cleanup-owned; no authored retry.
```

---

# Implementation style constraints

Prefer:

- fixed arrays/masks bounded by 15 keys;
- enums that make invalid states unrepresentable;
- `SmallVec<[T; MAX_KEYS]>` only when exact bounded identity lists are necessary;
- typed `QpcTicks`, `TimelineTicks`, `DurationTicks`;
- checked arithmetic;
- explicit structured errors for structural faults;
- RAII for Win32 handles/scheduling guards;
- pure core logic in `sky_dispatch_core`;
- Win32 bindings only in `sky_dispatch_win32`;
- thin `sky_player_rs` orchestration/FFI integration.

Avoid:

- new heap maps/priority queues for 15-key state;
- trait/dynamic-dispatch architecture for simple fixed paths;
- unsafe custom queues;
- duplicated old/new scheduler state machines;
- broad module movement during semantic PRs;
- compatibility fields influencing runtime decisions.

---

# Required behavior when implementation is difficult

Do not simplify by violating an invariant.

Examples of forbidden “solutions”:

### Problem: delayed Up shares timestamp with Down `[A, B]`

Forbidden:

```text
send B now
send A later
```

because the chord was split.

Required:

- if delayed Up is unrelated, defer only that Up and keep entire Down chord;
- if Down depends on delayed Up, fail closed.

### Problem: system stalls and next three notes are overdue

Forbidden:

```text
send all three immediately
```

Required:

apply backlog guard; do not perform second overdue physical send before a new future deadline wait.

### Problem: p99 start error is positive

Forbidden:

```text
next_target -= learned_error
```

Required:

measure, investigate wait/occupancy, preserve authored target.

### Problem: high-resolution timer unavailable

Forbidden:

```text
fall back to sleep and continue production
```

Required:

explicit failure/cleanup according to startup/runtime boundary.

---

# Tests are part of implementation

Do not postpone tests until the end.

Each semantic phase must add its regression tests at the same time.

Read `08_TEST_AND_BENCHMARK_MATRIX.md` before starting every phase.

For hot-path changes, capture before/after benchmark evidence on the same environment.

Mock/test emitters prove state-machine behavior only. They are not final Windows latency proof.

---

# Performance decision authority

Benchmarks can prove whether an approved change succeeded or regressed.

Benchmarks do **not** authorize you to invent a new controller or policy.

Examples:

- if 400 µs fixed spin benchmarks better than 700 µs, report it; do not silently change shipping constant from 700;
- if a custom queue seems faster, report the data; do not replace ArrayQueue;
- if TimeCritical appears faster in one run, report it; do not make it production default;
- if diagnostic pre-SendInput QPC sampling is cheap, report it; do not add it permanently to production without a new decision.

---

# Failure / contradiction protocol

Stop the current phase and report:

```text
Plan decision involved
Current code fact causing conflict
Exact files/functions
Why both cannot currently be satisfied
Smallest options that could resolve it
Tests/evidence already gathered
```

Do not choose an option yourself when it changes a locked architecture decision.

Ordinary implementation details that do not alter semantics do not require escalation.

---

# Completion report format per phase

Use this structure:

```text
Phase:
Base SHA:
Final SHA:

Implemented:
- ...

Decisions satisfied:
- D-...

Files changed:
- ...

Tests added/updated:
- ...

Commands run:
- ...

Functional results:
- ...

Performance before:
- ...

Performance after:
- ...

Regressions/risks:
- ...

Deferred items explicitly allowed by plan:
- ...
```

Do not write “all good” without concrete evidence.

---

# Final instruction

Implement the system described by this plan, not the system you would have designed independently.

Where the plan says **KEEP**, preserve the behavior.
Where it says **REFACTOR**, implement the specified replacement.
Where it says **REMOVE**, eliminate it from production after migration is complete.
Where it says **REJECT**, do not introduce it.

The primary objective is not the smallest diff or the highest benchmark headline. The primary objective is a simple, auditable Rust dispatch core that preserves authored physical timing as far as Windows permits and fails closed rather than fabricating timing when it cannot.
