# Rust RT Dispatch Round 2 — Authoritative Refactor Plan

**Status:** APPROVED DESIGN / IMPLEMENTATION NOT STARTED  
**Repository baseline:** `main@bd5542dd01b6612eea0ebb48c0f6e7a27d8e690e`  
**Plan date:** 2026-08-16  
**Scope:** Rust real-time keyboard dispatch core and the minimum Python/native boundary changes required to preserve its timing contract.

## 1. Authority of this plan

This folder is the implementation specification for the next refactor of the real-time dispatch core.

The coding agent is an **executor**, not an architect. It MUST NOT change the decisions in this folder because another implementation looks easier, because an existing test encodes different behavior, or because an older document describes a conflicting design.

When this plan conflicts with an older migration/overhaul/archive plan, this folder wins for the work covered here. Existing normative product behavior outside this scope remains unchanged unless a file in this folder explicitly says otherwise.

If implementation reveals a genuine contradiction that cannot be resolved while preserving these invariants, the coding agent MUST stop that phase and report the contradiction. It MUST NOT invent a third behavior.

## 2. Priority order

Every implementation decision is evaluated in this order:

1. Preserve authored note timestamps and chord semantics.
2. Never create ambiguous physical keyboard state.
3. Keep the dispatch thread available for the next physical deadline as quickly as possible.
4. Keep Windows timing semantics truthful and evidence-based.
5. Reduce jitter/tail latency before optimizing averages.
6. Prefer bounded fixed-size Rust state over dynamic orchestration.
7. Avoid complexity that does not protect one of the above invariants.

A change that saves microseconds but can shift a note, split a chord, create a catch-up burst, or make timing evidence ambiguous is rejected.

## 3. Locked architectural decisions

The detailed rationale is in `00_DECISION_REGISTER.md`. The coding agent must treat these as fixed:

- Authored Down chords are atomic. A chord is never split to make one key earlier.
- Runtime release floors are **per key/generation**, not a deadline for an entire same-timestamp packet.
- A release that is unrelated to a Down chord must never head-of-line block that Down chord.
- A same-key dependency that makes a Down chord physically impossible at its authored deadline is fail-closed; the engine does not silently move the chord.
- Deterministically impossible same-key hold intervals are rejected before worker start.
- Production never emits a rapid catch-up burst after a large stall. Multiple overdue physical boundaries are a fidelity fault.
- The planner owns one frozen absolute `physical_target_qpc`; wait and dispatch consume that exact value.
- QPC remains the authoritative monotonic clock. No RDTSC, wall-clock scheduling, CPU affinity, or `KEYBDINPUT.time` scheduling.
- `final_admission_qpc` means what its name says. It is not called a `SendInput` syscall-entry timestamp.
- No learned dispatch lead, EMA/PID controller, onset estimator, or automatic sender-latency compensation is added.
- Production wait uses high-resolution waitable timer + interrupt event + bounded QPC spin.
- Production does not depend on `timeBeginPeriod`.
- Production spin policy becomes fixed and deterministic at **700 µs**. Wake calibration remains diagnostic evidence, not a controller.
- Default scheduling remains MMCSS `Games` + `AVRT_PRIORITY_HIGH` when available, with the existing safe fallback; HighQoS opt-out remains. Do not default to `TIME_CRITICAL` and do not pin a CPU core.
- The RT producer remains allocation-free and nonblocking. It does only the minimum state commit plus one bounded observation enqueue.
- Keep `crossbeam_queue::ArrayQueue` for now. Do not introduce a custom unsafe SPSC queue without a separate benchmark-driven decision.
- Full coordinator invariant scans are not part of the healthy release build dispatch path.
- Release/pending state uses fixed arrays/masks bounded by `MAX_KEYS = 15`.
- No active-playback epoch-rebase controller is introduced in this refactor. Large stalls fail closed instead of rewriting the timeline.
- Keep `panic = "unwind"`; cleanup after an unexpected panic is more important than a theoretical abort-path optimization.

## 4. Target runtime shape

```text
                NON-RT / PRE-DEADLINE
                         │
             compiled authored frames
                         │
          coordinator fixed-slot state
       (active generations + pending releases)
                         │
              build immutable plan
        ┌────────────────┴────────────────┐
        │ exact absolute target_qpc       │
        │ prepared fixed INPUT[]          │
        │ exact commit token              │
        │ control/target preflight proof  │
        └────────────────┬────────────────┘
                         │
════════════════ PRECISION REGION ════════════════
                         │
              high-res timer wake early
                         │
                  bounded QPC spin
                         │
                final safety gates
                         │
               final_admission_qpc
                         │
                     SendInput
                         │
             sendinput_completion_qpc
                         │
           minimal fixed-state commit
                         │
          nonblocking observation push
                         │
════════════════ END PRECISION REGION ════════════
                         │
                  observer thread
       telemetry / conversions / health / publish
```

## 5. Plan files

1. `00_DECISION_REGISTER.md` — every keep/refactor/remove/reject decision.
2. `01_INVARIANTS_AND_FAILURE_POLICY.md` — invariants and fail-closed behavior.
3. `02_P0_RELEASE_SCHEDULER_AND_FEASIBILITY.md` — remove packet-wide release head-of-line blocking.
4. `03_P0_OVERDUE_STALL_POLICY.md` — eliminate overdue catch-up bursts.
5. `04_P1_RT_PATH_MINIMIZATION.md` — type/state cleanup and precision-path reduction.
6. `05_P1_TIMING_BOUNDARIES_AND_TELEMETRY.md` — truthful QPC boundaries and observer ownership.
7. `06_P1_WINDOWS_WAIT_AND_SCHEDULING_POLICY.md` — Windows 11 wait/priority policy.
8. `07_P2_COMPATIBILITY_AND_DEAD_CODE.md` — compatibility cleanup after semantics are stable.
9. `08_TEST_AND_BENCHMARK_MATRIX.md` — mandatory proof and acceptance matrix.
10. `09_IMPLEMENTATION_SEQUENCE.md` — exact PR/phase order for the coding agent.
11. `10_FILES_TOUCH_MAP.md` — expected files/functions and explicit non-goals.
12. `11_WINDOWS11_REFERENCE_NOTES.md` — primary Microsoft API constraints used by the design.
13. `12_CODING_AGENT_HANDOFF.md` — mandatory executor entrypoint and stop/escalation contract.

## 6. Coding-agent operating contract

The coding agent MUST:

- Implement phases in the order in `09_IMPLEMENTATION_SEQUENCE.md`.
- Add the specified regression tests before or together with each semantic change.
- Keep each PR/commit focused on one phase.
- Run the gates listed in `08_TEST_AND_BENCHMARK_MATRIX.md`.
- Preserve fixed-size/no-allocation behavior across the precision region.
- Use checked typed tick arithmetic.
- Treat any zero/partial/ambiguous Down-bearing `SendInput` result as terminal.
- Preserve cleanup on every terminal path.
- Update normative docs when behavior changes.

The coding agent MUST NOT:

- Redesign packet/release semantics beyond this plan.
- Add retries to the production Down/Mixed path.
- Add an adaptive timing controller.
- Add CPU affinity, realtime process priority, `THREAD_PRIORITY_TIME_CRITICAL` defaults, hooks, injection, game memory access, or alternative input mechanisms.
- Replace safe Rust with custom `unsafe` queue/timer code for speculative speed.
- Use mock timing as final proof of Windows latency.
- weaken a failure into silent continuation merely to make tests pass.

## 7. Definition of completion

This refactor is complete only when all of the following are true:

- A delayed unrelated release cannot move an authored Down chord.
- A physically impossible retrigger/chord is rejected or fails closed rather than being silently delayed.
- A large scheduler/OS stall cannot produce a multi-note catch-up burst.
- Wait and dispatch use one identical frozen absolute QPC target.
- The healthy production path from deadline wake to `SendInput` contains no planning, packet construction, telemetry materialization, heap allocation, mutex acquisition, logging, or full-state scan.
- Production spin behavior is deterministic and no longer controlled by a 32-sample startup probe.
- Timing field names describe their actual measurement boundaries.
- Existing cleanup/focus/command correctness remains intact.
- Windows real-host acceptance data shows no regression in p99/p99.9 dispatch-start error and no increase in integrity failures.

No performance claim is accepted without the evidence required by `08_TEST_AND_BENCHMARK_MATRIX.md`.
