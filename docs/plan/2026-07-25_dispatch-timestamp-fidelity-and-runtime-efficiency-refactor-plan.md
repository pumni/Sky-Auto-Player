# Plan: Dispatch Timestamp Fidelity and Runtime-Efficiency Refactor

> **Status:** PROPOSED — NOT IMPLEMENTED
>
> **Date:** 2026-07-25
>
> **Audience:** implementation AI agents and the final human/AI acceptance reviewer
>
> **Scope:** dispatch orchestration, real-time wait path, Windows `SendInput`
> boundary, timing telemetry, bounded runtime state, and directly owning
> normative documentation
>
> **Source:** deep review of the current dispatch and key-sending path on
> 2026-07-25

This document is a working proposal, not a normative architecture source.
`AGENTS.md`, `SECURITY.md`, `pyproject.toml`, `.python-version`, and the
normative P2 documents listed by `docs/INDEX.md` take precedence over every
statement in this plan. If implementation evidence conflicts with this plan,
stop, document the conflict, and follow the higher-priority source.

Do not change this file to `IMPLEMENTED` merely because code was written.
Implementation is complete only after all applicable gates and evidence in
this plan are supplied to the acceptance reviewer. The implementation agent
must not self-approve the result.

## 1. Outcome

Refactor the dispatch path so that:

1. An exact-timestamp chord remains one logical action and one contiguous
   `SendInput` batch when `chord_stagger_us == 0`.
2. Sender-side completion is measured as close as possible to the return of
   the real Win32 `SendInput` call.
3. The adaptive lead model learns from that correctly located timestamp,
   without mixing clock domains.
4. Cold-path preparation happens immediately before the final precision wait,
   not before a long blocking sleep.
5. There is no avoidable allocation, lock acquisition, disk I/O, or repeated
   capability probe on the per-dispatch hot path.
6. All state retained across playback is demonstrably bounded or explicitly
   cleared.
7. The design uses Python 3.14 free-threaded synchronization patterns whose
   correctness does not depend on undocumented incidental atomicity.
8. Documentation describes the attainable guarantee precisely: the project
   minimizes sender-side dispatch error; it cannot guarantee the exact frame
   in which the game observes an input.

The implementation must preserve current CLI behavior, security boundaries,
pause/stop responsiveness, release-before-press ordering, failure handling,
and supported timing profiles unless a phase below explicitly authorizes a
documented behavior change.

## 2. Immutable safety and architecture guardrails

The implementation agent must restate and check these before starting every
phase that touches the dispatch or Windows backend:

- Never modify game files.
- Never read game memory.
- Never attach a debugger, hook, inject, patch, or tamper with the game or any
  game process.
- Never bypass or evade anti-cheat.
- Use Windows `SendInput` only for simulated input.
- Do not add `python-keyboard`, `pynput`, `SetWindowsHookEx`, third-party
  keyboard packages, or another input mechanism.
- Validate every untrusted scan code and public input strictly.
- Keep `domain/` and `orchestration/` pure: no Win32, `ctypes`, wall clock, or
  concrete `SendInput` dependency.
- Keep all Win32 and `ctypes` code in `src/sky_music/platform/`.
- Infrastructure may bridge orchestration to platform but must not move Win32
  types into orchestration.
- Do not change `.python-version` or `requires-python`.
- Do not edit `scripts/audit_security_mandates.py`,
  `.config/security_audit_baseline.json`, `Sky-Auto-Player.spec`,
  `installer/updater.ps1`, committed golden schedules, or
  `perf-baselines/*` without explicit user permission.
- Do not add a dependency unless the user explicitly approves it and it is
  added with `uv add`.
- Use `uv run` for every Python command.

Additional timing invariants:

- The completion timestamp, not call-entry time, remains the scheduling anchor.
- Pending key releases remain higher priority than new presses when due.
- A partial note-on send may receive at most the currently documented
  immediate retry; do not sleep and retry a late note-on.
- A chord must not be split into per-key adaptive scheduling decisions.
- `chord_stagger_us == 0` is the fidelity mode. A non-zero stagger is an
  explicit remote/network tradeoff, not an accuracy optimization.
- No benchmark result may justify weakening strict validation or P0 mandates.
- Do not change same-key feasibility semantics solely from a theoretical
  argument. Phase 6 defines the evidence and approval gate.

## 3. Measurement model and terminology

Use the following timestamps consistently:

- `T_authored`: timestamp from the compiled song schedule.
- `T_dispatch_target`: sender-side deadline after the selected lead policy.
- `T_call_entry`: instant immediately before invoking Win32 `SendInput`.
- `T_call_return`: first timestamp sampled immediately after `SendInput`
  returns to Python.
- `T_game_observed`: unknown instant when the game consumes the input state.
- `sender_error_us = T_call_return - T_authored` for the current
  completion-anchored contract.

`T_game_observed` is not available from the application under the security
model. It includes OS queueing and game frame/input sampling uncertainty.
Production metrics, UI labels, tests, and documentation must not rename
sender-side error as game-observed or audio-onset accuracy.

Microsoft documents that events supplied in one successful `SendInput` call
are inserted serially and are not interspersed with other keyboard or mouse
events. This is the useful guarantee for chords. It is not hardware
simultaneity and must not be described as atomic simultaneous delivery.

For an exact-timestamp chord, the target model is:

```text
compile once
  -> one KeyAction(T_authored, [all chord scan codes])
  -> wait until T_dispatch_target
  -> one SendInput array/call
  -> sample T_call_return immediately
  -> update one chord-size lead bucket
```

No per-key lead calculation or wait is allowed inside that path.

## 4. Confirmed review findings that this plan addresses

These are implementation hypotheses backed by current code inspection. Each
must be converted into a failing regression test before its production fix.

| ID | Finding | Risk |
|---|---|---|
| F1 | `_last_send_completed_us` stores playback elapsed time while the warmup decision compares it with raw `clock.now_us()` | Mixed clock domains can classify nearly every dispatch as cold after epoch rebasing |
| F2 | The warmup hook executes before the long blocking wait | Cache/branch preparation can be stale by the real deadline |
| F3 | `CORE_WARMUP_SPIN_US` is 200 µs but the wired default budget is 500 µs | Behavior and normative timing description disagree |
| F4 | The first mid-song clock reprobe can occur shortly after the pre-play probe | Avoidable 16 ms class probe work can appear early in playback |
| F5 | `send_completed_us` is sampled by the backend after the platform wrapper has already returned | Lead learning includes avoidable Python return/bookkeeping time |
| F6 | Trusted batch validation constructs `set(scan_codes)` on every dispatch | Avoidable allocation and hashing on the hottest path |
| F7 | Progress counters acquire a cross-thread lock after each non-deferred dispatch | Free-threaded contention can add sender jitter |
| F8 | Debug telemetry can synchronously flush a large CSV when its hard cap is reached | A bounded memory safeguard can create an unbounded real-time stall |
| F9 | Relative waitable-timer duration is calculated before setup and armed later | Stale remaining time can wake later than intended |
| F10 | `auto` priority falls back from MMCSS to `TIME_CRITICAL` before `HIGHEST` | Aggressive fallback can contend with the game and system |
| F11 | Some shared-state reasoning relies on tuple/bool assignment being incidentally atomic | Python 3.14 free-threaded correctness should use documented synchronization |
| F12 | Existing same-key feasibility uses minimum hold only and warns about sub-frame off gaps | Repeated same-key visibility may depend on game frame sampling, but this is not proven |
| F13 | Current comments may overstate one-batch chord delivery as atomic | Incorrect guarantees obscure the actual accuracy boundary |

The review found no demonstrated unbounded leak in the normal path. Schedule
storage is proportional to song size, live-key state is proportional to
polyphony, reusable input-array caches are bounded and cleared, and telemetry
has a hard cap. Phase 7 must preserve these properties and distinguish retained
Python objects from allocator/RSS retention.

## 5. Required execution protocol

The implementation AI must:

1. Work one phase at a time.
2. Read every phase's owning P2 documents before editing.
3. Capture the pre-change behavior and commands in an implementation log.
4. Add a discriminating failing test before each behavior fix.
5. Apply the minimum code change that makes the new test pass.
6. Run the phase-local tests and gates.
7. Review the diff for unrelated changes.
8. Update the owning normative document in the same phase when behavior
   changes.
9. Stop on any approval gate instead of silently expanding scope.
10. Preserve evidence for final acceptance.

Do not combine an architectural refactor and a behavior change in one
unreviewable patch. If commits are requested later, use one conventional
commit per logical phase. Do not commit or push unless the user asks.

## 6. Phase map

| Phase | Purpose | Production behavior change |
|---|---|---|
| 0 | Freeze baseline and add regression characterization | No |
| 1 | Repair cold guard/warmup and clock reprobe timing | Yes |
| 2 | Move completion timestamp to the Win32 call boundary | Yes |
| 3 | Remove avoidable hot-path allocations and progress locking | Internal only |
| 4 | Correct waitable-timer arming and command/timer edges | Yes |
| 5 | Harden priority and free-threaded synchronization | Yes/internal |
| 6 | Resolve same-key frame visibility through external evidence | Gated |
| 7 | Eliminate hot-path telemetry I/O and prove bounded memory | Yes/internal |
| 8 | Reconcile terminology, documentation, and dead paths | Documentation/internal |
| 9 | Full validation, performance evidence, and handoff | No |

Phases 1–5 may be implemented independently only where their tests prove there
is no semantic coupling. Phase 2 must land before final adaptive-lead
benchmarks. Phase 6 must not block safe fixes in other phases, but its status
must be reported honestly as accepted, rejected, or awaiting human evidence.

## 7. Phase 0 — Baseline freeze and regression characterization

### Goal

Create reproducible evidence for current correctness, latency distribution,
CPU cost, and retained runtime state without changing production behavior.

### Read first

- `docs/INDEX.md`
- `docs/architecture.md`
- `docs/rt-dispatch-architecture.md`
- `docs/timing-principles.md`
- `docs/timing-profile-frame-model.md`
- `SECURITY.md`

### Required work

1. Record:
   - current commit;
   - dirty-worktree paths, preserving unrelated user changes;
   - Python version;
   - `sys._is_gil_enabled()` when available;
   - timing profile and benchmark machine context;
   - whether the benchmark used fake clocks or real Windows timers.
2. Run the existing scheduler, runtime loop, wait strategy, Windows backend,
   timing estimator, telemetry, and security tests.
3. Add characterization tests for these contracts:
   - exact timestamp chord compiles to one action;
   - the default chord path calls `SendInput` once with all members;
   - chord scan-code order is deterministic;
   - adaptive lead is selected once per chord-size bucket;
   - completion anchoring is used for subsequent timing;
   - release-before-press ordering remains unchanged;
   - pause/stop command sources preempt a future deadline within the documented
     bound;
   - array caches and telemetry buffers remain bounded and are cleared at the
     existing lifecycle boundary.
4. Add failing regression tests for F1–F11, but do not weaken current passing
   tests merely to make a failure visible.
5. Produce a five-run baseline for representative cases:
   - one key;
   - 2-, 4-, and maximum-supported-key chords;
   - dense same-timestamp chord sequences;
   - long silent gaps followed by a chord;
   - a due release immediately followed by a press;
   - active progress/UI observation;
   - telemetry disabled and enabled.

Store transient benchmark output outside committed `perf-baselines/`. Do not
add or edit an immutable baseline without user permission.

### Minimum regression tests to introduce

Names may follow nearby conventions, but each semantic must be explicit:

- `test_warmup_cold_detection_uses_elapsed_clock_domain_after_epoch_rebase`
- `test_cold_guard_is_adjacent_to_final_spin_after_long_gap`
- `test_default_cold_guard_budget_is_200_us`
- `test_first_mid_song_reprobe_is_not_due_before_interval`
- `test_send_completion_is_sampled_at_platform_call_return_boundary`
- `test_exact_timestamp_chord_uses_one_sendinput_call`
- `test_single_key_trusted_batch_avoids_duplicate_set_allocation`
- `test_progress_publication_does_not_lock_per_dispatch`
- `test_telemetry_cap_never_flushes_on_dispatch_thread`
- `test_waitable_timer_recomputes_remaining_immediately_before_arm`
- `test_auto_priority_never_selects_time_critical_fallback`

Avoid over-mocking implementation details. A test must fail for the reviewed
defect and remain valid after a small internal refactor.

### Exit criteria

- Existing behavior is characterized.
- Every proposed behavior change has a failing test.
- Baseline results include raw samples, median, p95, p99, maximum, and dispatch
  thread CPU time where available.
- No production file changed.

## 8. Phase 1 — Cold guard/warmup and clock reprobe repair

### Goal

Make the cold-dispatch guard effective at the deadline, use one clock domain,
and avoid an unnecessary early mid-song clock probe.

### Primary files

- `src/sky_music/orchestration/core/loop.py`
- the infrastructure wait bridge only if the existing hook cannot be placed
  correctly without changing its contract
- directly owning tests
- `docs/rt-dispatch-architecture.md`
- `docs/timing-principles.md`

### Required design

1. Store and compare last-send completion in playback elapsed microseconds, or
   store and compare both sides in raw monotonic time. Do not mix them.
2. Prefer one elapsed-time domain inside the pure scheduler.
3. Replace the early standalone warmup with a cold guard that expands the
   final active-spin window immediately before the deadline:

```text
cold = elapsed_since_last_completed_send >= cold_gap_threshold
final_guard_us = normal_spin_threshold_us + (200 if cold else 0)
block/event-wait until target - final_guard_us
perform final guard work
spin only for the remaining bounded interval
dispatch
```

4. The additional default cold guard is exactly 200 µs unless new same-machine
   benchmark evidence justifies a documented value.
5. Do not spend the guard when:
   - the dispatch is already late;
   - a command requires pause/stop;
   - a due release must run first;
   - no blocking interval remains.
6. The final guard must not touch game state, allocate an unbounded object, do
   disk I/O, or call `SendInput`.
7. Initialize/record the pre-play clock probe so that the first periodic
   reprobe is not eligible until the documented interval, normally at least
   30 seconds of playback elapsed time.
8. Remove unused constants or duplicate hooks only after all call sites and
   tests prove they are dead.

### Required tests

- Epoch-rebased fake clock with a large raw monotonic origin.
- Long silent gap proving guard execution is adjacent to the final spin.
- Short gap proving no cold guard.
- Already-late dispatch proving no extra 200 µs penalty.
- Pause/stop during the blocking interval.
- Release due at the same target.
- Pre-play probe followed by gaps at interval minus one microsecond, exactly
  the interval, and interval plus one microsecond.

### Acceptance

- No mixed clock-domain subtraction remains.
- The cold guard is bounded and adjacent to the precision wait.
- The first periodic reprobe respects the full interval.
- Relevant unit tests and scheduler tests pass.
- P2 timing docs match the implemented algorithm.

### Rollback trigger

Rollback or redesign this phase if p99 sender error regresses beyond the Phase
9 threshold, pause latency increases beyond its documented bound, or CPU time
shows that the guard executes outside cold transitions.

## 9. Phase 2 — Timestamp at the real `SendInput` return boundary

### Goal

Capture completion at the closest legal point after Win32 `SendInput` returns,
then propagate that timestamp without taking a second later clock sample.

### Primary files

- `src/sky_music/platform/win32/inputs.py`
- Windows input backend/interface implementation
- platform/backend tests
- runtime loop tests
- timing estimator/telemetry tests
- owning P2 documents

### Required design

1. Introduce a small typed result at the platform seam containing:
   - requested input count;
   - successfully inserted input count;
   - the timestamp sampled immediately after the native call returns;
   - Win32 error information when required by the existing contract.
2. Inject the monotonic clock callable into the platform wrapper/backend
   through the existing boundary. Do not import infrastructure into platform
   and do not expose Win32 types to orchestration.
3. In the normal full-send path:

```text
inserted = user32.SendInput(...)
completed_us = clock_now_us()  # immediately next observable operation
validate/branch/bookkeep using inserted and completed_us
```

4. Avoid logging, tuple reconstruction, error formatting, telemetry, or a
   backend-level second clock read between the native return and
   `completed_us`.
5. Define partial-send semantics explicitly:
   - capture a timestamp after every native attempt;
   - the logical batch completion is the final attempted call's return;
   - preserve the existing immediate-retry limit;
   - never claim a partially inserted chord was atomic;
   - expose enough count/error data for the existing safe-abort path.
6. Feed the platform-captured completion to:
   - completion anchoring;
   - adaptive lead observation;
   - telemetry;
   - progress accounting where relevant.
7. Do not change authored timestamps or calculate separate timestamps for chord
   members.

### Required tests

- Fake native function and sequenced fake clock proving the first clock call
  after native return becomes the result timestamp.
- Backend test proving it does not overwrite the platform timestamp.
- Full, zero, and partial insertion cases.
- Immediate partial retry with first and final completion timestamps.
- One-key and multi-key batches.
- Adaptive bucket observation using the final result timestamp.
- No wall-clock import in `domain/` or `orchestration/`.

### Acceptance

- Exactly one native call and one immediate completion sample in the successful
  normal path.
- No later timestamp substitutes for the platform sample.
- Partial behavior remains bounded and explicitly reported.
- Windows and scheduler-purity security audits pass.
- P2 docs use the same completion definition.

## 10. Phase 3 — Remove avoidable hot-path allocation and locking

### Goal

Keep strict validation while making the normal dispatch path allocation-light
and removing cross-thread progress locking from each send.

### Primary files

- `src/sky_music/platform/win32/inputs.py`
- schedule/compiler validation seam
- playback supervisor/progress sink
- runtime loop observer seam
- directly owning tests

### 10.1 Trusted input validation

1. Public/untrusted APIs must retain complete range, type, duplicate, and
   batch-size validation.
2. Establish and test an ahead-of-time invariant that compiled `KeyAction`
   scan-code tuples are:
   - immutable;
   - within the allowed scan-code range;
   - within maximum chord size;
   - duplicate-free.
3. At the trusted platform seam:
   - provide an explicit one-key fast path;
   - avoid `set(scan_codes)` allocation;
   - for small tuples, use allocation-free pairwise duplicate validation if
     defense in depth is still required;
   - do not add a mutable global validation cache.
4. Reusable ctypes input arrays must remain bounded by supported batch size
   and cleared at the established lifecycle boundary.

### 10.2 Progress publication

1. Dispatch-local counters belong to the dispatch thread.
2. Replace per-dispatch cross-thread lock acquisition with one of:
   - local aggregation published at the existing 30–50 ms snapshot cadence;
   - an already-established bounded single-producer handoff;
   - another simpler documented primitive proven by tests and benchmarks.
3. Do not create an unbounded queue.
4. Do not make progress exactness delay `SendInput`; UI counters may lag by one
   bounded publication interval.
5. Force a final publication on pause, stop, normal completion, and error so
   displayed totals converge.
6. Focus and command signals are correctness controls, not progress telemetry;
   do not batch them in a way that weakens safety.

### Optional cleanup

Remove redundant `Queue.empty()` followed by `get_nowait()` only if a focused
test proves command behavior and the change stays local.

### Required tests

- Strict public validation rejects invalid, duplicate, and oversized inputs.
- Trusted single-key path does not construct a set.
- Maximum chord remains validated and sends once.
- Progress update count is much lower than dispatch count during dense input.
- Pause, stop, normal completion, and error publish final exact counters.
- Publication state is bounded under a long synthetic song.
- UI reader and free-threaded writer stress test shows no torn semantic state.

### Acceptance

- No per-dispatch progress lock in the normal send path.
- No per-dispatch duplicate-set allocation.
- Strict validation remains at every untrusted boundary.
- Dense benchmark shows no correctness regression and no CPU regression.

## 11. Phase 4 — Waitable-timer arming and command/timer edge correction

### Goal

Remove stale relative-timer calculations and prove deterministic behavior when
a timer and a control command become ready together.

### Primary files

- `src/sky_music/infrastructure/wait_strategy.py`
- Windows waitable-timer wrapper only if its interface lacks required data
- runtime/wait tests
- owning timing docs

### Required design

1. Compute the intended blocking cutoff from the system-clock target and guard.
2. Perform reusable setup.
3. Immediately before arming a relative waitable timer, sample the clock again:

```text
sleep_us = target_system_us - clock.now_us() - final_guard_us
```

4. If `sleep_us <= 0`, do not arm a timer for stale or negative time; proceed
   to command check/final precision wait according to the existing contract.
5. Keep command-event wakeup in the blocking wait.
6. Add a discriminating test for simultaneous timer and command readiness.
   Define the safe result explicitly:
   - a pending pause/stop must be observed before a new note-on;
   - already-due release cleanup must remain safe;
   - no busy retry loop.
7. Preserve bounded fallback behavior on timer creation/arming/wait failure.
   Never spin for an entire long gap after a recoverable platform failure.
8. Do not add per-deadline handle creation if reusable handles already exist.

### Required tests

- Fake clock advances during setup before arm.
- Deadline becomes due before arm.
- Command arrives before timer.
- Command and timer become ready simultaneously.
- Spurious event wake.
- Timer API failure.
- Very short remaining interval bypasses blocking wait.

### Acceptance

- Relative duration uses a clock sample adjacent to arming.
- Pause/stop cannot lose a tie to a new note-on.
- Release safety and bounded CPU fallback remain intact.
- Real Windows timer tests pass where available.

## 12. Phase 5 — Priority and Python 3.14 free-threaded hardening

### Goal

Use conservative real-time priority fallback and documented synchronization
under the pinned free-threaded interpreter.

### Primary files

- infrastructure dispatch-thread creation
- MMCSS/thread-priority bridge
- focus signal
- playback display snapshot
- relevant tests
- `docs/architecture.md`
- `docs/rt-dispatch-architecture.md`

### Required design

1. Change `auto` dispatch priority order to:

```text
MMCSS Pro Audio/appropriate task
  -> THREAD_PRIORITY_HIGHEST
  -> ordinary priority/off with an explicit diagnostic
```

2. `THREAD_PRIORITY_TIME_CRITICAL` may remain only as an explicit user-selected
   expert mode if already supported. It must not be an automatic fallback.
3. Never raise process priority.
4. Create the dispatch thread with an explicit empty `contextvars.Context`
   through Python 3.14's `threading.Thread(context=...)` support, so the
   free-threaded default cannot inherit irrelevant caller context.
5. Replace custom cross-thread boolean signaling with a documented primitive
   such as `threading.Event` where semantically appropriate.
6. A fresh foreground-window probe immediately before sending remains the
   authoritative focus safety check.
7. Replace any correctness claim based only on tuple/bool assignment atomicity
   with documented synchronization. Display-only snapshot locking may occur at
   pause/publication transitions, but not once per dispatched note.
8. Do not add a lock around the native `SendInput` call.

### Required tests

- Auto mode selects MMCSS on success.
- Auto mode selects `HIGHEST`, never `TIME_CRITICAL`, when MMCSS fails.
- Explicit `TIME_CRITICAL` behavior, if retained, requires explicit config.
- Complete priority failure degrades safely.
- Dispatch thread receives an empty context.
- Focus signal set/clear/read stress test.
- Fresh focus probe still blocks a send when the cached signal is stale.
- Display snapshots remain internally consistent.

### Acceptance

- Priority docs and code agree.
- No auto `TIME_CRITICAL`.
- Cross-thread correctness relies on documented primitives.
- No new per-note lock or allocation is introduced.

## 13. Phase 6 — Evidence-gated same-key/frame visibility decision

### Goal

Determine whether a repeated same key needs a minimum observable key-up window
for the game to recognize the second press. Do not assume the answer.

### Why this phase is gated

Current normative behavior treats a same-key interval as feasible when it
satisfies minimum hold and warns, rather than rejects, when the release-to-
repress gap is shorter than a frame. Earlier design work rejected changing
this contract without evidence. Normative documents therefore remain binding
until new external evidence and user approval justify a change.

### Prohibited methods

- No game memory reads.
- No hooks or injection.
- No debugger attach.
- No anti-cheat interaction.
- No process instrumentation.
- No hidden FPS detection from the game.

### Allowed evidence

- Pure schedule simulations.
- Sender-side dispatch telemetry.
- Human observation.
- Externally captured audio/onset data, if separately approved and implemented
  without game/process tampering.
- Screen recording analyzed after the fact, if it does not hook the game.

### Experiment protocol

1. First build a pure simulator that reports for every repeated key:
   - first key-down timestamp;
   - key-up timestamp;
   - next key-down timestamp;
   - effective hold;
   - effective off gap;
   - assumed frame duration for display only.
2. Generate test material around boundaries:
   - zero off gap;
   - 0.25 frame;
   - 0.5 frame;
   - 1.0 frame;
   - 1.5 frames;
   - at least two representative frame rates, including 60 Hz and a high
     refresh scenario such as 144 Hz.
3. Keep chord size, profile, device, and network mode fixed within a run.
4. Perform enough repetitions to distinguish a systematic miss from a single
   observation. Record the chosen sample count before inspecting results.
5. Record:
   - schedule;
   - sender-side telemetry;
   - configured profile/frame assumption;
   - observable success/failure;
   - machine and game mode;
   - limitations.

### Decision table

| Evidence | Action |
|---|---|
| No reliable miss correlated with short off gaps | Preserve current feasibility; document evidence and limitations |
| Reliable threshold requiring an up interval | Draft a separate behavior proposal for minimum off-gap semantics |
| Inconclusive/no human run | Mark Phase 6 `AWAITING EVIDENCE`; do not change production semantics |

If a minimum off gap is supported, stop and request user approval before:

- changing scheduler feasibility;
- changing timing-profile fields/defaults;
- updating golden schedules;
- changing warning/error CLI behavior.

An approved change must update `docs/timing-principles.md`,
`docs/timing-profile-frame-model.md`, scheduler edge tests, and any explicitly
approved goldens in one logical phase. It must specify whether an impossible
repeat is rejected, clamped, or warned; silent timing distortion is forbidden.

## 14. Phase 7 — Telemetry real-time safety and bounded-memory proof

### Goal

Ensure debug observability cannot trigger disk I/O on the dispatch thread and
prove that repeated playback does not create unbounded retained state.

### Primary files

- timing telemetry recorder/export path
- playback lifecycle cleanup
- array/cache owners
- tests and non-immutable benchmark utilities
- owning docs if behavior changes

### Required design

1. The dispatch thread may append only to a bounded in-memory telemetry
   structure when telemetry is enabled.
2. Reaching the hard cap must:
   - stop accepting additional detail records or overwrite according to an
     explicitly documented bounded policy;
   - increment an exact dropped/truncated count;
   - record one logical truncation marker;
   - never synchronously write CSV from the dispatch path.
3. Flush/export only at an existing non-real-time lifecycle point such as
   pause completion, playback end, or explicit user export.
4. Prefer bounded truncation over adding a new writer thread unless measured
   requirements prove a writer is necessary.
5. If a writer thread is justified later, its queue must be bounded and
   shutdown/join behavior must be tested. It must never block dispatch.
6. Repeated-session tests must cover:
   - normal completion;
   - pause/resume;
   - stop;
   - backend error;
   - telemetry cap;
   - maximum chord-size cache use.
7. Assert bounded live objects/capacity and lifecycle cleanup. Do not require
   process RSS to return exactly to baseline; Python 3.14 free-threaded and its
   allocator may retain arenas without a live-object leak.

### Required tests

- Hard cap reached during dense synthetic playback without file I/O.
- Exact truncation/dropped count.
- One deferred export after playback.
- Export failure does not corrupt dispatch completion.
- N repeated sessions have stable retained list/dict/cache sizes after warmup.
- Input-array cache remains bounded by supported batch sizes.
- Playback-owned references are released after success and error.

### Acceptance

- No filesystem operation is reachable from the per-dispatch record call.
- Runtime storage has an explicit bound.
- Repeated sessions show no monotonic retained-object growth attributable to
  playback.
- Telemetry summaries disclose truncation honestly.

## 15. Phase 8 — Documentation, terminology, and focused cleanup

### Goal

Make code, tests, and normative documentation state the same guarantees and
remove only dead paths created by this refactor.

### Required work

1. Replace claims of chord “atomicity” with the precise guarantee:
   one contiguous `SendInput` batch whose inserted events are serial and not
   interspersed on a successful call.
2. Define sender-side completion error separately from game-observed timing.
3. Document:
   - corrected cold guard placement/budget;
   - completion timestamp boundary;
   - auto priority ladder;
   - progress publication cadence;
   - telemetry truncation policy;
   - same-key decision or unresolved evidence gate.
4. Remove only constants, hooks, branches, or tests made obsolete by completed
   phases.
5. Keep `docs/INDEX.md` accurate. Do not mark this proposal implemented until
   the final reviewer accepts the evidence.
6. Do not rewrite historical plan files or immutable performance baselines.

### Acceptance

- P2 documents describe implemented behavior.
- No comment promises `T_game_observed`.
- No stale 500 µs/200 µs warmup contradiction remains.
- No unrelated documentation is reformatted.

## 16. Phase 9 — Final validation and acceptance handoff

### 16.1 Mandatory gates

Run from the repository root:

```powershell
uv run ruff check .
uv run pyright
uv run pytest
uv run --env-file .env python scripts/audit_security_mandates.py
uv run --env-file .env python scripts/audit_free_threaded_wheels.py
```

Also run focused Windows tests on Windows 11. If a gate cannot run, do not
claim completion; provide the exact failure and root-cause analysis.

A release build is not automatically required for this source refactor.
If packaging/build files change or the user requests release confidence, run
the documented `build_app` gate with `.env`; do not use `--skip-test`.

### 16.2 Correctness acceptance

- Exact-timestamp chord: one `KeyAction`, one lead decision, one native
  `SendInput` call in the full-success default path.
- All chord members preserve deterministic ordering.
- Completion timestamp is sampled immediately after native return and reused.
- No clock-domain mixing.
- Release-before-press behavior remains correct.
- Pause/stop cannot lose a simultaneous-ready edge to a new note-on.
- Partial sends remain bounded and safe.
- Strict validation and all P0 security mandates pass.
- Same-key semantics are unchanged unless Phase 6 evidence and approval exist.

### 16.3 Performance acceptance

Compare five-run before/after distributions on the same machine and setup.
Correctness gates are hard requirements; hardware timing metrics are
comparative evidence, not universal constants.

The default regression threshold is:

- no statistically consistent worsening of median sender error;
- p99 must not worsen by more than the larger of 100 µs or 10% of the baseline
  absolute p99, unless explained and explicitly accepted;
- dispatch-thread CPU time must not increase for dense playback;
- long-gap CPU use must remain bounded by the selected timer plus final guard;
- progress/telemetry work per dispatch must decrease or remain absent;
- no additional native call per exact-timestamp chord.

Report raw results even when the threshold passes. Do not select only favorable
runs. If ambient system load invalidates a comparison, rerun the complete
before/after set under the same documented conditions.

### 16.4 Memory acceptance

- No unbounded queue/cache/list in playback.
- Repeated-session retained object counts stabilize after warmup.
- Array caches and telemetry buffers remain within declared caps.
- All playback-owned state is cleared on completion, stop, and error.
- RSS alone is not used as proof of a leak or proof of no leak.

### 16.5 Acceptance packet

The implementation AI must provide the reviewer:

1. Final changed-file list and reason for every file.
2. Phase-by-phase summary.
3. Tests added, including which test failed before each fix.
4. Exact outputs of mandatory gates.
5. Before/after raw benchmark data and summarized statistics.
6. One-key and chord telemetry samples showing timestamp relationships.
7. Evidence that there is no extra `SendInput` call for a chord.
8. Security audit output.
9. Memory/lifecycle evidence.
10. Phase 6 evidence and decision, or an explicit `AWAITING EVIDENCE` status.
11. All deviations from this plan and their higher-priority justification.
12. Remaining risks and recommended follow-up, without hiding failed or
    inconclusive results.

The reviewer should reject the implementation if the packet contains only
aggregate claims without raw commands/results, if P2 docs disagree with code,
or if the AI changed same-key behavior without the Phase 6 gate.

## 17. File-impact guide

This is a navigation guide, not blanket authorization.

| Area | Expected reason to edit |
|---|---|
| `src/sky_music/orchestration/core/loop.py` | clock domain, final cold guard, reprobe timing, result propagation |
| `src/sky_music/infrastructure/wait_strategy.py` | adjacent timer calculation, timer/command edge |
| playback supervisor/progress sink | batched counter publication |
| dispatch-thread infrastructure | context and synchronization |
| MMCSS/thread priority bridge | conservative auto fallback |
| `src/sky_music/platform/win32/inputs.py` | immediate completion timestamp and allocation-light trusted validation |
| telemetry module | bounded truncation and deferred export |
| directly owning tests | regression and lifecycle evidence |
| P2 timing/architecture docs | same-change behavior documentation |

Any edit outside these areas requires a written scope explanation. Dependency,
interpreter, build-spec, updater, security-audit, golden, and immutable
performance-baseline changes require the approvals stated in `AGENTS.md`.

## 18. Suggested logical change sequence

If the user later requests commits, keep them focused:

1. `test(scheduler): characterize dispatch timing boundaries`
2. `fix(scheduler): place cold guard at final dispatch wait`
3. `fix(windows): capture send completion at native boundary`
4. `refactor(scheduler): batch progress publication`
5. `refactor(windows): remove trusted batch allocation`
6. `fix(scheduler): recompute wait duration before timer arm`
7. `fix(windows): use conservative dispatch priority fallback`
8. `fix(telemetry): defer capped trace export off dispatch`
9. `docs(scheduler): align dispatch accuracy guarantees`

Do not force this split when a test and its minimal production fix would become
separated. Never mix unrelated cleanup into these changes.

## 19. Abort and escalation conditions

Stop the affected phase and ask the user when:

- a fix requires changing a P0 rule;
- a change would move Win32/`ctypes` outside platform;
- the interpreter pin or core dependency would need to change;
- a golden schedule or immutable performance baseline must change;
- same-key production semantics appear to require modification;
- externally observed game evidence requires a new capture mechanism;
- a benchmark suggests using hooks, injection, process inspection, or another
  forbidden technique;
- strict validation appears incompatible with a proposed optimization;
- unrelated dirty files overlap the required change;
- mandatory validation cannot run after root-cause investigation.

For an ordinary failing test or difficult performance result, investigate and
fix the cause; do not classify normal engineering difficulty as an exception.

## 20. Final definition of done

The refactor is ready for independent acceptance only when all applicable
items are true:

- [ ] Every behavior fix began with a discriminating failing test.
- [ ] Exact timestamp chords still use one batch/call in fidelity mode.
- [ ] Sender completion is captured at the native return boundary.
- [ ] The cold guard uses one clock domain and runs adjacent to final spin.
- [ ] The first periodic reprobe respects the full interval.
- [ ] No duplicate-set allocation exists in the trusted normal path.
- [ ] No per-dispatch progress lock exists.
- [ ] No dispatch-thread telemetry file I/O exists.
- [ ] Waitable-timer duration is recomputed immediately before arming.
- [ ] Auto priority never falls back to `TIME_CRITICAL`.
- [ ] Free-threaded shared state uses documented synchronization.
- [ ] Runtime buffers/caches are bounded and lifecycle-tested.
- [ ] Same-key behavior is evidence-gated and honestly reported.
- [ ] P2 documents match implementation.
- [ ] Ruff, pyright, pytest, security audit, and free-threaded audit pass.
- [ ] Before/after timing, CPU, and memory evidence is attached.
- [ ] No unrelated files or dependencies changed.
- [ ] The acceptance packet is delivered to the independent reviewer.

Only after the reviewer accepts this checklist and its evidence should
`docs/INDEX.md` and this plan's status be changed from `PROPOSED` to
`IMPLEMENTED`.

## 21. Primary references

- `AGENTS.md`
- `SECURITY.md`
- `docs/INDEX.md`
- `docs/architecture.md`
- `docs/rt-dispatch-architecture.md`
- `docs/timing-principles.md`
- `docs/timing-profile-frame-model.md`
- Microsoft `SendInput` documentation:
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput>
- Microsoft Multimedia Class Scheduler Service documentation:
  <https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service>
- Microsoft thread priority documentation:
  <https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority>
- Microsoft waitable timer documentation:
  <https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw>
- Python 3.14 free-threading guide:
  <https://docs.python.org/3.14/howto/free-threading-python.html>
- Python 3.14 `threading` documentation:
  <https://docs.python.org/3.14/library/threading.html>
- Python 3.14 `time` documentation:
  <https://docs.python.org/3.14/library/time.html>
